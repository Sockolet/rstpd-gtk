use crate::{
    core::Result,
    launch::{self, Request},
};
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const SOCKET_NAME: &std::ffi::CStr = c"instance.sock";

fn open_ipc_dir(directory: &Path, create: bool) -> std::io::Result<fs::File> {
    let ipc = directory.join("ipc");
    if create {
        match DirBuilder::new().mode(0o700).create(&ipc) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let meta = fs::symlink_metadata(&ipc)?;
        if !meta.is_dir() {
            return Err(std::io::Error::other(
                "IPC path must be a directory, not a symlink or file.",
            ));
        }
        fs::set_permissions(&ipc, fs::Permissions::from_mode(0o700))?;
    }
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&ipc)?;
    let meta = dir.metadata()?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "IPC directory must be a private (0700) directory owned by the current user.",
        ));
    }
    Ok(dir)
}

fn socket_path(dir: &fs::File) -> PathBuf {
    // Using /proc/self/fd avoids Linux's 108-byte sun_path limit and anchors lookup to the verified dirfd.
    PathBuf::from(format!("/proc/self/fd/{}/instance.sock", dir.as_raw_fd()))
}

fn remove_stale_socket(dir: &fs::File) -> Result<()> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            SOCKET_NAME.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(());
        }
        return Err(format!("Could not inspect existing IPC socket: {error}"));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFSOCK {
        return Err("IPC socket path exists and is not a Unix domain socket.".into());
    }
    if unsafe { libc::unlinkat(dir.as_raw_fd(), SOCKET_NAME.as_ptr(), 0) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("Could not remove stale IPC socket: {error}"));
        }
    }
    Ok(())
}

fn check_peer(stream: &UnixStream) -> Result<()> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of_val(&credentials) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::from_mut(&mut credentials).cast(),
            &mut size,
        )
    } != 0
    {
        return Err(format!(
            "Could not verify the file launch peer: {}",
            std::io::Error::last_os_error()
        ));
    }
    if size as usize != std::mem::size_of_val(&credentials)
        || credentials.uid != unsafe { libc::geteuid() }
    {
        return Err("File launches must come from the same user.".into());
    }
    Ok(())
}

struct Pending {
    stream: UnixStream,
    bytes: Vec<u8>,
    deadline: Instant,
}

impl Pending {
    fn read_request(&mut self) -> Result<Option<Request>> {
        if Instant::now() >= self.deadline {
            return Err("Timed out receiving a file launch request.".into());
        }
        loop {
            let total = if self.bytes.len() < 4 {
                4
            } else {
                let length = u32::from_le_bytes(self.bytes[..4].try_into().unwrap()) as usize;
                if length > launch::MAX_REQUEST_BYTES {
                    return Err("The file launch request exceeds 64 KiB.".into());
                }
                length + 4
            };
            if self.bytes.len() == total && total != 4 {
                return Request::decode(&self.bytes[4..]).map(Some);
            }
            if self.bytes.len() == 4 && total == 4 {
                return Err("The file launch request is empty.".into());
            }
            let mut buffer = [0; 4096];
            let remaining = (total - self.bytes.len()).min(buffer.len());
            match self.stream.read(&mut buffer[..remaining]) {
                Ok(0) => {
                    return Err("The file launch sender disconnected before completion.".into());
                }
                Ok(count) => self.bytes.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(format!("Could not receive the file launch: {error}")),
            }
        }
    }
}

pub struct Server {
    ipc_dir: fs::File,
    listener: UnixListener,
    pending: Vec<Pending>,
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            libc::unlinkat(self.ipc_dir.as_raw_fd(), SOCKET_NAME.as_ptr(), 0);
        }
    }
}

impl Server {
    pub fn new(directory: &Path) -> Result<Self> {
        let ipc_dir = open_ipc_dir(directory, true)
            .map_err(|error| format!("Could not prepare private IPC directory: {error}"))?;
        remove_stale_socket(&ipc_dir)?;
        let path = socket_path(&ipc_dir);
        let listener = UnixListener::bind(&path)
            .map_err(|error| format!("Could not register file launch forwarding: {error}"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not secure IPC socket permissions: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            ipc_dir,
            listener,
            pending: Vec::new(),
        })
    }

    pub fn poll(&mut self) -> Vec<Result<Request>> {
        let mut results = Vec::new();
        for _ in self.pending.len()..launch::MAX_PENDING_REQUESTS {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Err(error) = check_peer(&stream).and_then(|()| {
                        stream
                            .set_nonblocking(true)
                            .map_err(|error| error.to_string())
                    }) {
                        results.push(Err(error));
                    } else {
                        self.pending.push(Pending {
                            stream,
                            bytes: Vec::new(),
                            deadline: Instant::now() + Duration::from_secs(5),
                        });
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    results.push(Err(format!("Could not accept the file launch: {error}")));
                    break;
                }
            }
        }
        let mut index = 0;
        while index < self.pending.len() {
            let result = self.pending[index].read_request();
            if matches!(result, Ok(None)) {
                index += 1;
                continue;
            }
            let mut pending = self.pending.remove(index);
            let accepted = u8::from(result.is_ok());
            let ack = pending.stream.write_all(&[accepted]);
            match (result, ack) {
                (Ok(Some(request)), Ok(())) => results.push(Ok(request)),
                (Ok(Some(_)), Err(error)) => results.push(Err(format!(
                    "Could not acknowledge the file launch: {error}"
                ))),
                (Err(error), _) => results.push(Err(error)),
                (Ok(None), _) => unreachable!(),
            }
        }
        results
    }
}

pub fn forward(directory: &Path, request: &Request) -> Result<()> {
    let bytes = request.encode()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        let ipc_dir = match open_ipc_dir(directory, false) {
            Ok(dir) => dir,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "The session is in use, but its editor is not ready for file launches: {error}. Close an older version first, or retry after startup finishes."
                ));
            }
        };
        match UnixStream::connect(socket_path(&ipc_dir)) {
            Ok(stream) => break stream,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                return Err(format!(
                    "The session is in use, but its editor is not ready for file launches: {error}. Close an older version first, or retry after startup finishes."
                ));
            }
        }
    };
    check_peer(&stream)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(5))))
        .and_then(|()| stream.write_all(&(bytes.len() as u32).to_le_bytes()))
        .and_then(|()| stream.write_all(&bytes))
        .map_err(|error| format!("Could not forward the file launch: {error}"))?;
    let mut accepted = [0];
    stream.read_exact(&mut accepted).map_err(|error| {
        format!("The running editor did not acknowledge the file launch: {error}")
    })?;
    if accepted != [1] {
        return Err("The running editor rejected the file launch.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_requests_do_not_block_and_complete_only_after_the_full_frame() {
        let (mut sender, receiver) = UnixStream::pair().unwrap();
        receiver.set_nonblocking(true).unwrap();
        check_peer(&receiver).unwrap();
        let request = Request::new(vec!["unicode-\u{65e5}.txt".into()], vec![], vec![]).unwrap();
        let bytes = request.encode().unwrap();
        let mut pending = Pending {
            stream: receiver,
            bytes: vec![],
            deadline: Instant::now() + Duration::from_secs(5),
        };
        sender
            .write_all(&(bytes.len() as u32).to_le_bytes()[..2])
            .unwrap();
        assert!(pending.read_request().unwrap().is_none());
        sender
            .write_all(&(bytes.len() as u32).to_le_bytes()[2..])
            .unwrap();
        sender.write_all(&bytes[..5]).unwrap();
        assert!(pending.read_request().unwrap().is_none());
        sender.write_all(&bytes[5..]).unwrap();
        assert_eq!(pending.read_request().unwrap(), Some(request));
    }

    #[test]
    fn oversized_and_expired_requests_are_rejected_without_blocking() {
        let (mut sender, receiver) = UnixStream::pair().unwrap();
        receiver.set_nonblocking(true).unwrap();
        let mut pending = Pending {
            stream: receiver,
            bytes: vec![],
            deadline: Instant::now() + Duration::from_secs(5),
        };
        sender
            .write_all(&((launch::MAX_REQUEST_BYTES + 1) as u32).to_le_bytes())
            .unwrap();
        assert!(pending.read_request().is_err());
        pending.deadline = Instant::now();
        assert!(pending.read_request().is_err());
    }

    #[test]
    fn private_ipc_socket_handles_long_paths_stale_sockets_and_cleanup() {
        let base = std::env::temp_dir().join(format!(
            "rstpd-ipc-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // Exceed Linux's 108-byte sun_path limit to ensure dirfd-anchored binding works.
        let deep = base.join("a".repeat(60)).join("b".repeat(60));
        fs::create_dir_all(&deep).unwrap();
        let mut server = Server::new(&deep).unwrap();
        let ipc_meta = fs::symlink_metadata(deep.join("ipc")).unwrap();
        assert_eq!(ipc_meta.mode() & 0o777, 0o700);
        let sock_meta = fs::symlink_metadata(deep.join("ipc/instance.sock")).unwrap();
        assert_eq!(sock_meta.mode() & 0o777, 0o600);
        let request = Request::new(vec!["sample.txt".into()], vec![], vec![]).unwrap();
        let dir_for_client = deep.clone();
        let expected = Request::new(vec!["sample.txt".into()], vec![], vec![]).unwrap();
        let client = std::thread::spawn(move || forward(&dir_for_client, &request));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = Vec::new();
        while received.is_empty() && Instant::now() < deadline {
            received.extend(server.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        client.join().unwrap().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received.remove(0).unwrap(), expected);
        // Simulate abrupt crash leaving a stale socket file behind.
        std::mem::forget(server);
        assert!(deep.join("ipc/instance.sock").exists());
        let clean = Server::new(&deep).unwrap();
        drop(clean);
        assert!(!deep.join("ipc/instance.sock").exists());
        fs::remove_dir_all(&base).unwrap();
    }
}

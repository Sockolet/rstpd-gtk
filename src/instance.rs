use crate::{
    core::Result,
    launch::{self, Request},
};
use std::{
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixListener, UnixStream},
    },
    path::Path,
    time::{Duration, Instant},
};

fn address(directory: &Path) -> Result<SocketAddr> {
    let name = format!(
        "rstpd-gtk.{}.{}",
        unsafe { libc::geteuid() },
        launch::session_key(directory)?
    );
    SocketAddr::from_abstract_name(name).map_err(|error| error.to_string())
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
    listener: UnixListener,
    pending: Vec<Pending>,
}

impl Server {
    pub fn new(directory: &Path) -> Result<Self> {
        let listener = UnixListener::bind_addr(&address(directory)?)
            .map_err(|error| format!("Could not register file launch forwarding: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        Ok(Self {
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
            match pending.stream.write_all(&[accepted]) {
                Ok(()) => {
                    if let Ok(Some(request)) = result {
                        results.push(Ok(request));
                    } else if let Err(error) = result {
                        results.push(Err(error));
                    }
                }
                Err(error) => results.push(Err(format!(
                    "Could not acknowledge the file launch: {error}"
                ))),
            }
        }
        results
    }
}

pub fn forward(directory: &Path, request: &Request) -> Result<()> {
    let address = address(directory)?;
    let bytes = request.encode()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match UnixStream::connect_addr(&address) {
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
}

use crate::core::{EditorFont, Encoding, Eol, MAX_DOCUMENT_BYTES, Result};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, fchown};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const SESSION_VERSION: u32 = 2;

/// Uses an absolute XDG state root, or an absolute HOME when XDG is unset/relative.
#[cfg(target_os = "linux")]
pub fn linux_default_directory() -> Result<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME");
    let home = std::env::var_os("HOME");
    linux_directory_from_paths(
        state.as_deref().map(Path::new),
        home.as_deref().map(Path::new),
    )
}

#[cfg(target_os = "linux")]
fn linux_directory_from_paths(state: Option<&Path>, home: Option<&Path>) -> Result<PathBuf> {
    if let Some(state) = state.filter(|path| path.is_absolute()) {
        return Ok(state.join("rstpd-gtk"));
    }
    // XDG requires relative (including empty) values to be ignored.
    let home = home
        .filter(|path| path.is_absolute())
        .ok_or("Recovery requires an absolute XDG_STATE_HOME or HOME directory.")?;
    Ok(home.join(".local/state/rstpd-gtk"))
}

/// Keeps the advisory lock until dropped. The stable lock file is never unlinked.
#[cfg(target_os = "linux")]
pub struct SessionLock {
    _file: fs::File,
}

#[cfg(target_os = "linux")]
fn create_private_directories(directory: &Path) -> Result<()> {
    match fs::metadata(directory) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => return Err(format!("{} is not a directory.", directory.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect {}: {error}",
                directory.display()
            ));
        }
    }
    let parent = parent_directory(directory)?;
    create_private_directories(parent)?;
    match fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(|error| {
                format!("Could not make {} private: {error}", directory.display())
            })?;
            if fs::metadata(directory)
                .map_err(|error| error.to_string())?
                .mode()
                & 0o777
                != 0o700
            {
                return Err(format!(
                    "{} does not support private directory permissions.",
                    directory.display()
                ));
            }
            for path in [directory, parent] {
                fs::File::open(path)
                    .and_then(|file| file.sync_all())
                    .map_err(|error| format!("Could not flush {}: {error}", path.display()))?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::metadata(directory)
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                return Err(format!("{} is not a directory.", directory.display()));
            }
        }
        Err(error) => {
            return Err(format!("Could not create {}: {error}", directory.display()));
        }
    }
    Ok(())
}

/// Creates missing directories as 0700 without chmod'ing existing ancestors, then
/// takes a nonblocking exclusive lock. Keep the returned guard for the whole session.
#[cfg(target_os = "linux")]
pub fn lock_directory(directory: &Path) -> Result<SessionLock> {
    let operation = || -> Result<SessionLock> {
        create_private_directories(directory)?;
        let metadata = fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
        if !metadata.is_dir() {
            return Err(
                "Recovery directory must be a directory, not a symlink or another file type."
                    .into(),
            );
        }
        if metadata.mode() & 0o022 != 0 {
            return Err("Recovery directory must not be writable by other users.".into());
        }
        let parent = fs::File::open(directory).map_err(|error| error.to_string())?;
        if !same_linux_file(
            &metadata,
            &parent.metadata().map_err(|error| error.to_string())?,
        ) {
            return Err("Recovery directory changed while opening it.".into());
        }
        let path = directory.join("session.lock");
        let file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => {
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .map_err(|error| error.to_string())?;
                file
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let before = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
                if !before.is_file() || before.nlink() != 1 {
                    return Err(
                        "Session lock must be a regular, single-link file, not a symlink.".into(),
                    );
                }
                let file = OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .map_err(|error| error.to_string())?;
                if !same_linux_file(
                    &before,
                    &file.metadata().map_err(|error| error.to_string())?,
                ) {
                    return Err("Session lock changed while opening it.".into());
                }
                file
            }
            Err(error) => return Err(format!("Could not create the session lock: {error}")),
        };
        let locked_metadata = file.metadata().map_err(|error| error.to_string())?;
        if locked_metadata.uid() != metadata.uid() {
            return Err("Session lock and recovery directory must have the same owner.".into());
        }
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(
                    "Another rstpd-gtk session is already using this recovery directory.".into(),
                );
            }
            Err(fs::TryLockError::Error(error)) => {
                return Err(format!("Could not acquire the session lock: {error}"));
            }
        }
        let current = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if !current.is_file()
            || current.nlink() != 1
            || !same_linux_file(&locked_metadata, &current)
        {
            return Err("Session lock changed while acquiring it.".into());
        }
        file.sync_all()
            .map_err(|error| format!("Could not flush the session lock: {error}"))?;
        parent
            .sync_all()
            .map_err(|error| format!("Could not flush the recovery directory: {error}"))?;
        Ok(SessionLock { _file: file })
    };
    operation().map_err(|error| format!("Could not lock {}: {error}", directory.display()))
}

pub fn default_directory(local_app_data: &Path) -> Result<PathBuf> {
    let current = local_app_data.join("rstpd");
    let legacy = local_app_data.join("RSTPad");
    for directory in [&current, &legacy] {
        for name in ["session.json", "session.lock"] {
            let path = directory.join(name);
            if path.try_exists().map_err(|error| {
                format!(
                    "Could not inspect recovery state {}: {error}",
                    path.display()
                )
            })? {
                return Ok(directory.clone());
            }
        }
    }
    Ok(current)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentSnapshot {
    pub id: u64,
    pub title: String,
    pub path: Option<PathBuf>,
    pub text: String,
    pub encoding: Encoding,
    pub eol: Eol,
    pub language: String,
    pub dirty: bool,
    pub disk_hash: Option<u64>,
    pub caret: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub documents: Vec<DocumentSnapshot>,
    pub active: usize,
    pub theme: String,
    #[serde(default)]
    pub editor_font: EditorFont,
    #[serde(default)]
    pub custom_languages: Vec<crate::udl::UserLanguage>,
    #[serde(default)]
    pub completion_api: Vec<crate::completion::Api>,
}

/// FNV-1a (64-bit). Pinned on purpose: this value is written to the recovery file and
/// compared after a restart, so it must not change when the Rust toolchain changes.
/// `DefaultHasher` gives no such guarantee. Not cryptographic; used only to detect
/// that a file changed outside the editor.
pub fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("{} exceeds the size limit.", path.display()));
    }
    Ok(bytes)
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "linux")]
fn parent_directory(path: &Path) -> Result<&Path> {
    path.parent()
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            }
        })
        .ok_or_else(|| "Destination has no parent directory.".into())
}

#[cfg(target_os = "linux")]
fn same_linux_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(target_os = "linux")]
fn linux_save_destination(
    path: &Path,
    follow_symlinks: bool,
) -> Result<(PathBuf, Option<fs::Metadata>)> {
    match fs::symlink_metadata(path) {
        Ok(original) => {
            if original.is_symlink() && !follow_symlinks {
                return Err(
                    "Recovery files must not be symlinks; the linked file has not been changed."
                        .into(),
                );
            }
            let destination = fs::canonicalize(path).map_err(|error| {
                format!("Could not resolve the destination (possibly a dangling symlink): {error}")
            })?;
            let metadata = fs::symlink_metadata(&destination).map_err(|error| error.to_string())?;
            if !metadata.is_file() {
                return Err("Destination must be a regular file.".into());
            }
            if !original.is_symlink() && !same_linux_file(&original, &metadata) {
                return Err(
                    "Destination changed while resolving it; the file has not been replaced."
                        .into(),
                );
            }
            if metadata.nlink() != 1 {
                return Err("Cannot atomically save a hard-linked file without breaking its links. Use Save As.".into());
            }
            if metadata.mode() & 0o7000 != 0 {
                return Err(
                    "Cannot safely replace a file with special permission bits. Use Save As."
                        .into(),
                );
            }
            if metadata.mode() & 0o200 == 0 {
                return Err(
                    "Destination is read-only for its owner; the file has not been replaced."
                        .into(),
                );
            }
            Ok((destination, Some(metadata)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path.file_name().ok_or("Destination has no file name.")?;
            let parent = fs::canonicalize(parent_directory(path)?)
                .map_err(|error| format!("Could not resolve the destination directory: {error}"))?;
            Ok((parent.join(name), None))
        }
        Err(error) => Err(format!("Could not inspect the destination: {error}")),
    }
}

#[cfg(target_os = "linux")]
fn linux_verify_destination(
    path: &Path,
    destination: &Path,
    previous: Option<&fs::Metadata>,
    follow_symlinks: bool,
) -> Result<()> {
    let (current_path, current) = linux_save_destination(path, follow_symlinks)?;
    let unchanged = match (previous, current.as_ref()) {
        (None, None) => true,
        (Some(before), Some(after)) => {
            same_linux_file(before, after)
                && before.uid() == after.uid()
                && before.gid() == after.gid()
                && before.mode() == after.mode()
                && before.size() == after.size()
                && before.mtime() == after.mtime()
                && before.mtime_nsec() == after.mtime_nsec()
                && before.ctime() == after.ctime()
                && before.ctime_nsec() == after.ctime_nsec()
        }
        _ => false,
    };
    if current_path != destination || !unchanged {
        return Err("Destination changed while saving; the file has not been replaced. Reload it or use Save As.".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_publish_new_file(staging: &Path, destination: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let staging = CString::new(staging.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    // The owned C strings outlive the syscall; no glibc 2.28 wrapper is required.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            staging.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Atomically saves bytes. Linux preserves ordinary mode bits and ownership,
/// follows existing document symlinks, and flushes the file and its directory.
/// Unsafe existing targets (including hard-linked files) are rejected.
/// New files require kernel/filesystem support for renameat2(RENAME_NOREPLACE);
/// unsupported systems fail rather than using a non-atomic or overwriting fallback.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write_impl(path, bytes, true)
}

fn atomic_write_impl(path: &Path, bytes: &[u8], explicit_save: bool) -> Result<()> {
    #[cfg(target_os = "linux")]
    let (destination, previous) = linux_save_destination(path, explicit_save)
        .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
    #[cfg(not(target_os = "linux"))]
    let destination = {
        let _ = explicit_save;
        path
    };
    let parent = destination
        .parent()
        .ok_or("Destination has no parent directory.")?;
    #[cfg(target_os = "linux")]
    let directory = fs::File::open(parent)
        .map_err(|error| format!("Could not open the destination directory: {error}"))?;
    let tmp = parent.join(format!(
        ".rstpd-{}-{}.tmp",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut created = false;
    let mut operation = || -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(target_os = "linux")]
        options.mode(0o600);
        let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
        created = true;
        #[cfg(target_os = "linux")]
        {
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
            let replacement = file.metadata().map_err(|error| error.to_string())?;
            if replacement.mode() & 0o777 != 0o600 {
                return Err(
                    "The filesystem does not support private temporary-file permissions.".into(),
                );
            }
            if let Some(previous) = &previous {
                if previous.uid() != replacement.uid() {
                    return Err(
                        "Cannot safely replace a file owned by another user. Use Save As.".into(),
                    );
                }
                if previous.gid() != replacement.gid() {
                    fchown(&file, None, Some(previous.gid())).map_err(|error| {
                        format!("Could not preserve the destination group: {error}")
                    })?;
                    if file.metadata().map_err(|error| error.to_string())?.gid() != previous.gid() {
                        return Err("The filesystem did not preserve the destination group.".into());
                    }
                }
            }
        }
        file.write_all(bytes).map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        if let Some(previous) = &previous
            && explicit_save
        {
            file.set_permissions(fs::Permissions::from_mode(previous.mode() & 0o777))
                .map_err(|error| format!("Could not preserve destination permissions: {error}"))?;
            if file.metadata().map_err(|error| error.to_string())?.mode() & 0o777
                != previous.mode() & 0o777
            {
                return Err("The filesystem did not preserve destination permissions.".into());
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
            };
            let from: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let ok = unsafe {
                if path.exists() {
                    ReplaceFileW(
                        to.as_ptr(),
                        from.as_ptr(),
                        std::ptr::null(),
                        0,
                        std::ptr::null(),
                        std::ptr::null(),
                    )
                } else {
                    MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH)
                }
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        #[cfg(all(not(windows), not(target_os = "linux")))]
        fs::rename(&tmp, destination).map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        {
            linux_verify_destination(path, &destination, previous.as_ref(), explicit_save)?;
            if previous.is_some() {
                fs::rename(&tmp, &destination).map_err(|error| error.to_string())?;
            } else {
                linux_publish_new_file(&tmp, &destination).map_err(|error| {
                    format!("Could not atomically create the new file with renameat2(RENAME_NOREPLACE): {error}")
                })?;
            }
        }
        created = false;
        #[cfg(target_os = "linux")]
        directory.sync_all().map_err(|error| {
            format!("File replaced, but parent-directory flush failed; durability is not confirmed: {error}")
        })?;
        Ok(())
    };
    let result = operation();
    if let Err(error) = &result
        && created
        && let Err(cleanup) = fs::remove_file(&tmp)
        && cleanup.kind() != std::io::ErrorKind::NotFound
    {
        return Err(format!(
            "Could not save {}: {error}; temporary-file cleanup failed: {cleanup}",
            path.display()
        ));
    }
    result.map_err(|e| format!("Could not save {}: {e}", path.display()))
}

pub fn load(path: &Path) -> Result<Session> {
    let exists = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(target_os = "linux")]
            if metadata.is_symlink() {
                return Err(
                    "Recovery files must not be symlinks; the linked file has not been changed."
                        .into(),
                );
            }
            #[cfg(not(target_os = "linux"))]
            let _ = metadata;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(format!(
                "Could not inspect recovery file {}: {error}",
                path.display()
            ));
        }
    };
    if !exists {
        return Ok(Session {
            version: SESSION_VERSION,
            theme: "system".into(),
            ..Session::default()
        });
    }
    let bytes = read_bounded(path, MAX_DOCUMENT_BYTES * 2)?;
    let session: Session = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Recovery file is invalid; it has not been changed: {e}"))?;
    if !matches!(session.version, 1 | SESSION_VERSION) {
        return Err("Unsupported recovery version; the file has not been changed.".into());
    }
    if session.documents.len() > 256
        || session
            .documents
            .iter()
            .any(|d| d.text.len() > MAX_DOCUMENT_BYTES)
    {
        return Err("Recovery file exceeds document limits; it has not been changed.".into());
    }
    if session.custom_languages.len() > 64 || session.completion_api.len() > 10_000 {
        return Err("Recovery language/completion definitions exceed their limits.".into());
    }
    for language in &session.custom_languages {
        language.validate()?;
    }
    for api in &session.completion_api {
        if api.language.len() > 128
            || api.receiver.len() > 256
            || api.name.len() > 256
            || api.signature.len() > 4096
            || api.name.contains('\0')
            || api.signature.contains('\0')
        {
            return Err("Recovery contains an invalid completion definition.".into());
        }
    }
    Ok(session)
}

pub fn save(path: &Path, session: &Session) -> Result<()> {
    struct BoundedOutput(Vec<u8>);
    impl Write for BoundedOutput {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len() + bytes.len() > MAX_DOCUMENT_BYTES * 2 {
                return Err(std::io::Error::other(
                    "Recovery exceeds 256 MiB. Save and close some documents.",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = BoundedOutput(Vec::new());
    serde_json::to_writer(&mut output, session).map_err(|e| e.to_string())?;
    atomic_write_impl(path, &output.0, false)
}

pub struct RecoveryWorker {
    tx: std::sync::mpsc::Sender<Option<(u64, Session)>>,
    pub rx: std::sync::mpsc::Receiver<(u64, Result<()>)>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl RecoveryWorker {
    pub fn new(path: PathBuf) -> Self {
        let (tx, requests) = std::sync::mpsc::channel::<Option<(u64, Session)>>();
        let (results, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            while let Ok(Some((revision, session))) = requests.recv() {
                if results.send((revision, save(&path, &session))).is_err() {
                    break;
                }
            }
        });
        Self {
            tx,
            rx,
            thread: Some(thread),
        }
    }
    pub fn submit(&self, revision: u64, session: Session) -> Result<()> {
        self.tx
            .send(Some((revision, session)))
            .map_err(|_| "Recovery worker stopped.".into())
    }
    pub fn flush(&self, revision: u64, session: Session) -> Result<()> {
        self.submit(revision, session)?;
        loop {
            let (saved, result) = self.rx.recv().map_err(|_| "Recovery worker stopped.")?;
            if saved == revision {
                return result;
            }
        }
    }
}
impl Drop for RecoveryWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(None);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renamed_app_preserves_legacy_recovery_and_lock_locations() {
        let root = std::env::temp_dir().join(format!(
            "rstpd-rename-test-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let current = root.join("rstpd");
        let legacy = root.join("RSTPad");
        assert_eq!(default_directory(&root).unwrap(), current);
        fs::create_dir(&legacy).unwrap();
        fs::write(legacy.join("session.lock"), b"").unwrap();
        assert_eq!(default_directory(&root).unwrap(), legacy);
        fs::create_dir(&current).unwrap();
        assert_eq!(default_directory(&root).unwrap(), legacy);
        fs::write(legacy.join("session.json"), b"legacy recovery").unwrap();
        assert_eq!(default_directory(&root).unwrap(), legacy);
        fs::write(current.join("session.json"), b"current recovery").unwrap();
        assert_eq!(default_directory(&root).unwrap(), current);
        assert_eq!(
            fs::read(legacy.join("session.json")).unwrap(),
            b"legacy recovery"
        );
        for file in [
            current.join("session.json"),
            legacy.join("session.json"),
            legacy.join("session.lock"),
        ] {
            fs::remove_file(file).unwrap();
        }
        fs::remove_dir(current).unwrap();
        fs::remove_dir(legacy).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn atomic_session_round_trip_and_replace() {
        let dir = std::env::temp_dir().join(format!(
            "rstpd-test-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("session.json");
        let mut session = Session {
            version: 1,
            theme: "dark".into(),
            ..Session::default()
        };
        save(&path, &session).unwrap();
        session.documents.push(DocumentSnapshot {
            id: 1,
            title: "Untitled".into(),
            path: None,
            text: "unsaved\0text".into(),
            encoding: Encoding::Utf8,
            eol: Eol::Lf,
            language: "Rust".into(),
            dirty: true,
            disk_hash: None,
            caret: 3,
        });
        save(&path, &session).unwrap();
        assert_eq!(load(&path).unwrap().documents[0].text, "unsaved\0text");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;
        use std::{
            ffi::OsString,
            os::unix::{ffi::OsStringExt, fs::symlink},
            process::Command,
        };

        struct TestDirectory(PathBuf);
        impl TestDirectory {
            fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "rstpd-linux-test-{}-{}",
                    std::process::id(),
                    TEMP_ID.fetch_add(1, Ordering::Relaxed)
                ));
                fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
                Self(path)
            }
            fn path(&self, name: &str) -> PathBuf {
                self.0.join(name)
            }
            fn assert_no_temporary_files(&self) {
                for entry in fs::read_dir(&self.0).unwrap() {
                    let name = entry.unwrap().file_name();
                    assert!(!name.to_string_lossy().starts_with(".rstpd-"), "{name:?}");
                }
            }
        }
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }

        #[test]
        fn editor_font_is_optional_persistent_and_rejects_invalid_recovery() {
            let root = TestDirectory::new();
            let path = root.path("session.json");
            for version in [1, SESSION_VERSION] {
                let legacy = serde_json::json!({
                    "version": version, "documents": [], "active": 0, "theme": "system"
                });
                fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
                let mut restored = load(&path).unwrap();
                assert_eq!(restored.editor_font, EditorFont::default());
                restored.editor_font = EditorFont::new("Sans", 1250).unwrap();
                save(&path, &restored).unwrap();
                assert_eq!(load(&path).unwrap().editor_font, restored.editor_font);
                assert_eq!(load(&path).unwrap().version, version);
            }
            for invalid in [
                serde_json::json!(null),
                serde_json::json!({}),
                serde_json::json!({"family": "", "size_hundredths": 1100}),
                serde_json::json!({"family": "Sans\0Mono", "size_hundredths": 1100}),
                serde_json::json!({"family": "Sans", "size_hundredths": 399}),
                serde_json::json!({"family": "Sans", "size_hundredths": 7201}),
                serde_json::json!({"family": "Sans", "size_hundredths": -1}),
                serde_json::json!({"family": "Sans", "size_hundredths": 12.5}),
            ] {
                let bytes = serde_json::to_vec(&serde_json::json!({
                    "version": SESSION_VERSION, "documents": [], "active": 0,
                    "theme": "system", "editor_font": invalid
                }))
                .unwrap();
                fs::write(&path, &bytes).unwrap();
                assert!(load(&path).is_err());
                assert_eq!(
                    fs::read(&path).unwrap(),
                    bytes,
                    "Invalid recovery must remain untouched"
                );
            }
            root.assert_no_temporary_files();
        }

        #[test]
        fn new_file_publication_moves_the_complete_staging_inode() {
            let root = TestDirectory::new();
            let staging = root.path("staging");
            let destination = root.path("new-document");
            fs::write(&staging, b"complete\0document\r\n").unwrap();
            fs::set_permissions(&staging, fs::Permissions::from_mode(0o600)).unwrap();
            fs::File::open(&staging).unwrap().sync_all().unwrap();
            let before = fs::metadata(&staging).unwrap();

            linux_publish_new_file(&staging, &destination).unwrap();

            let after = fs::metadata(&destination).unwrap();
            assert!(same_linux_file(&before, &after));
            assert_eq!(fs::read(&destination).unwrap(), b"complete\0document\r\n");
            assert_eq!(after.mode() & 0o777, 0o600);
            assert!(!staging.try_exists().unwrap());
            assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
        }

        #[test]
        fn new_file_publication_does_not_clobber_raced_in_destinations() {
            for kind in ["file", "symlink", "dangling-symlink", "directory"] {
                let root = TestDirectory::new();
                let staging = root.path("staging");
                let destination = root.path("destination");
                let external = root.path("external");
                fs::write(&staging, b"staged bytes").unwrap();
                fs::write(&external, b"external bytes").unwrap();
                let (resolved, previous) = linux_save_destination(&destination, true).unwrap();
                linux_verify_destination(&destination, &resolved, previous.as_ref(), true).unwrap();
                assert!(previous.is_none());

                match kind {
                    "file" => fs::write(&destination, b"raced-in bytes").unwrap(),
                    "symlink" => symlink("external", &destination).unwrap(),
                    "dangling-symlink" => symlink("missing", &destination).unwrap(),
                    "directory" => fs::create_dir(&destination).unwrap(),
                    _ => unreachable!(),
                }
                let before = fs::symlink_metadata(&destination).unwrap();
                let error = linux_publish_new_file(&staging, &destination).unwrap_err();
                assert_eq!(
                    error.kind(),
                    std::io::ErrorKind::AlreadyExists,
                    "{kind}: {error}"
                );
                assert!(same_linux_file(
                    &before,
                    &fs::symlink_metadata(&destination).unwrap()
                ));
                assert_eq!(fs::read(&staging).unwrap(), b"staged bytes");
                assert_eq!(fs::read(&external).unwrap(), b"external bytes");
                match kind {
                    "file" => assert_eq!(fs::read(&destination).unwrap(), b"raced-in bytes"),
                    "symlink" => {
                        assert_eq!(fs::read_link(&destination).unwrap(), Path::new("external"))
                    }
                    "dangling-symlink" => {
                        assert_eq!(fs::read_link(&destination).unwrap(), Path::new("missing"));
                        assert!(!root.path("missing").try_exists().unwrap());
                    }
                    "directory" => assert_eq!(fs::read_dir(&destination).unwrap().count(), 0),
                    _ => unreachable!(),
                }
            }
        }

        #[test]
        fn new_file_publication_failure_never_creates_an_empty_destination() {
            let root = TestDirectory::new();
            let staging = root.path("missing-staging");
            let destination = root.path("new-document");
            let error = linux_publish_new_file(&staging, &destination).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            assert!(!destination.try_exists().unwrap());
            assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);

            fs::write(&staging, b"staged bytes").unwrap();
            let error = linux_publish_new_file(&staging, &root.path("invalid\0name")).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
            assert_eq!(fs::read(&staging).unwrap(), b"staged bytes");
            assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
        }

        #[test]
        fn new_file_publication_preserves_non_utf8_paths() {
            let root = TestDirectory::new();
            let staging = root.0.join(OsString::from_vec(b"staging-\xfe".to_vec()));
            let destination = root.0.join(OsString::from_vec(b"document-\xff".to_vec()));
            fs::write(&staging, b"staged bytes").unwrap();
            linux_publish_new_file(&staging, &destination).unwrap();
            assert_eq!(fs::read(&destination).unwrap(), b"staged bytes");
            assert!(!staging.try_exists().unwrap());
        }

        #[test]
        fn default_paths_require_absolute_roots_and_follow_xdg_precedence() {
            let home = Path::new("/home/editor");
            let fallback = home.join(".local/state/rstpd-gtk");
            for state in [None, Some(Path::new("")), Some(Path::new("relative/state"))] {
                assert_eq!(
                    linux_directory_from_paths(state, Some(home)).unwrap(),
                    fallback
                );
            }
            for home in [None, Some(Path::new("relative/home")), Some(home)] {
                assert_eq!(
                    linux_directory_from_paths(Some(Path::new("/state")), home).unwrap(),
                    Path::new("/state/rstpd-gtk")
                );
            }
            for home in [None, Some(Path::new("")), Some(Path::new("relative/home"))] {
                assert!(linux_directory_from_paths(None, home).is_err());
                assert!(
                    linux_directory_from_paths(Some(Path::new("relative/state")), home).is_err()
                );
            }
            assert_eq!(
                linux_directory_from_paths(Some(Path::new("/")), None).unwrap(),
                Path::new("/rstpd-gtk")
            );
        }

        #[test]
        fn default_paths_preserve_non_utf8_components() {
            let home = PathBuf::from(OsString::from_vec(b"/home/editor-\xff".to_vec()));
            assert_eq!(
                linux_directory_from_paths(None, Some(&home)).unwrap(),
                home.join(".local/state/rstpd-gtk")
            );
            assert_eq!(
                linux_directory_from_paths(Some(&home), None).unwrap(),
                home.join("rstpd-gtk")
            );
        }

        #[test]
        fn save_path_resolution_handles_relative_file_names() {
            assert_eq!(
                parent_directory(Path::new("notes.txt")).unwrap(),
                Path::new(".")
            );
            assert_eq!(
                parent_directory(Path::new("dir/notes.txt")).unwrap(),
                Path::new("dir")
            );
            assert_eq!(
                parent_directory(Path::new("/notes.txt")).unwrap(),
                Path::new("/")
            );
            assert!(parent_directory(Path::new("/")).is_err());
            assert!(parent_directory(Path::new("")).is_err());
            let (path, metadata) = linux_save_destination(Path::new("Cargo.toml"), true).unwrap();
            assert!(path.is_absolute());
            assert!(metadata.unwrap().is_file());
        }

        #[test]
        fn recovery_is_private_and_preserves_document_metadata() {
            let root = TestDirectory::new();
            let directory = root.path("state");
            let _lock = lock_directory(&directory).unwrap();
            let path = directory.join("session.json");
            let mut session = Session {
                version: SESSION_VERSION,
                theme: "dark".into(),
                documents: vec![DocumentSnapshot {
                    id: 42,
                    title: "notes".into(),
                    path: Some(root.path("notes.txt")),
                    text: "unsaved\0\u{107}\r\nnext\r\n".into(),
                    encoding: Encoding::Legacy("IBM852".into()),
                    eol: Eol::CrLf,
                    language: "Rust".into(),
                    dirty: true,
                    disk_hash: Some(12345),
                    caret: 11,
                }],
                ..Session::default()
            };
            save(&path, &session).unwrap();
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
            session.documents[0].text.push_str("later");
            save(&path, &session).unwrap();
            let restored = load(&path).unwrap();
            let document = &restored.documents[0];
            assert_eq!(restored.version, SESSION_VERSION);
            assert_eq!(restored.theme, "dark");
            assert_eq!(document.text, session.documents[0].text);
            assert_eq!(document.path, session.documents[0].path);
            assert_eq!(document.encoding, session.documents[0].encoding);
            assert_eq!(document.eol, Eol::CrLf);
            assert!(document.dirty);
            assert_eq!(document.disk_hash, Some(12345));
            assert_eq!(document.caret, 11);
            assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        }

        #[test]
        fn recovery_replacement_does_not_keep_public_plaintext_permissions() {
            let root = TestDirectory::new();
            let path = root.path("session.json");
            let session = Session {
                version: SESSION_VERSION,
                theme: "system".into(),
                ..Session::default()
            };
            save(&path, &session).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            save(&path, &session).unwrap();
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
            assert_eq!(load(&path).unwrap().version, SESSION_VERSION);
        }

        #[test]
        fn explicit_saves_preserve_permissions_owner_and_bytes() {
            let root = TestDirectory::new();
            let path = root.path("document");
            let bytes = b"new\0contents\r\nwith\nmixed\rendings";
            for mode in [0o600, 0o640, 0o644, 0o664, 0o700, 0o751] {
                fs::write(&path, b"previous").unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
                let before = fs::metadata(&path).unwrap();
                atomic_write(&path, bytes).unwrap();
                let after = fs::metadata(&path).unwrap();
                assert_eq!(fs::read(&path).unwrap(), bytes);
                assert_eq!(after.mode() & 0o777, mode);
                assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
                assert_ne!(before.ino(), after.ino());
                root.assert_no_temporary_files();
            }
        }

        #[test]
        fn explicit_save_follows_relative_symlink_chains_without_replacing_links() {
            let root = TestDirectory::new();
            fs::create_dir(root.path("data")).unwrap();
            fs::create_dir(root.path("links")).unwrap();
            let target = root.path("data/notes");
            let first = root.path("links/first");
            let second = root.path("links/second");
            fs::write(&target, b"previous").unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
            symlink("../data/notes", &first).unwrap();
            symlink("first", &second).unwrap();
            let before = fs::symlink_metadata(&second).unwrap();
            atomic_write(&second, b"replacement").unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"replacement");
            assert_eq!(fs::read(&second).unwrap(), b"replacement");
            assert_eq!(fs::read_link(&first).unwrap(), Path::new("../data/notes"));
            assert_eq!(fs::read_link(&second).unwrap(), Path::new("first"));
            assert_eq!(fs::symlink_metadata(&second).unwrap().ino(), before.ino());
            assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o640);
            assert_eq!(fs::read_dir(root.path("data")).unwrap().count(), 1);
            assert_eq!(fs::read_dir(root.path("links")).unwrap().count(), 2);
        }

        #[test]
        fn dangling_looping_and_directory_symlinks_are_not_overwritten() {
            let root = TestDirectory::new();
            let dangling = root.path("dangling");
            let looping = root.path("loop");
            let directory = root.path("directory-link");
            symlink("missing-target", &dangling).unwrap();
            symlink("loop", &looping).unwrap();
            symlink(".", &directory).unwrap();
            for path in [&dangling, &looping, &directory] {
                assert!(atomic_write(path, b"must not escape").is_err());
                assert!(fs::symlink_metadata(path).unwrap().is_symlink());
            }
            assert!(!root.path("missing-target").exists());
            root.assert_no_temporary_files();
        }

        #[test]
        fn recovery_does_not_follow_symlinks_into_external_files() {
            let root = TestDirectory::new();
            let target = root.path("external");
            let path = root.path("session.json");
            let session = Session {
                version: SESSION_VERSION,
                theme: "dark".into(),
                ..Session::default()
            };
            save(&target, &session).unwrap();
            let before = fs::read(&target).unwrap();
            symlink("external", &path).unwrap();
            assert!(save(&path, &session).unwrap_err().contains("symlink"));
            assert!(load(&path).unwrap_err().contains("symlink"));
            assert_eq!(fs::read(&target).unwrap(), before);
            assert_eq!(fs::read_link(&path).unwrap(), Path::new("external"));
            root.assert_no_temporary_files();
        }

        #[test]
        fn hard_links_read_only_and_special_permission_files_are_not_replaced() {
            let root = TestDirectory::new();
            let original = root.path("original");
            let alias = root.path("alias");
            fs::write(&original, b"shared").unwrap();
            fs::hard_link(&original, &alias).unwrap();
            assert!(
                atomic_write(&original, b"new")
                    .unwrap_err()
                    .contains("hard-linked")
            );
            assert_eq!(fs::read(&original).unwrap(), b"shared");
            assert_eq!(fs::read(&alias).unwrap(), b"shared");
            assert_eq!(fs::metadata(&original).unwrap().nlink(), 2);
            for mode in [0o444, 0o4755, 0o2640] {
                let path = root.path(&format!("mode-{mode:o}"));
                fs::write(&path, b"protected").unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
                assert!(atomic_write(&path, b"new").is_err());
                assert_eq!(fs::read(&path).unwrap(), b"protected");
                assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, mode);
            }
            assert!(atomic_write(&root.0, b"not a file").is_err());
            root.assert_no_temporary_files();
        }

        #[test]
        fn changed_destinations_are_reported_without_overwriting_external_data() {
            let root = TestDirectory::new();
            let path = root.path("document");
            fs::write(&path, b"before").unwrap();
            let (destination, previous) = linux_save_destination(&path, true).unwrap();
            fs::write(&path, b"external edit with another size").unwrap();
            assert!(
                linux_verify_destination(&path, &destination, previous.as_ref(), true).is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), b"external edit with another size");

            let new = root.path("new");
            let (destination, previous) = linux_save_destination(&new, true).unwrap();
            fs::write(&new, b"created externally").unwrap();
            assert!(linux_verify_destination(&new, &destination, previous.as_ref(), true).is_err());
            assert_eq!(fs::read(&new).unwrap(), b"created externally");

            let link = root.path("link");
            symlink("document", &link).unwrap();
            let (destination, previous) = linux_save_destination(&link, true).unwrap();
            fs::remove_file(&link).unwrap();
            symlink("new", &link).unwrap();
            assert!(
                linux_verify_destination(&link, &destination, previous.as_ref(), true).is_err()
            );
            assert_eq!(fs::read(&new).unwrap(), b"created externally");
        }

        #[test]
        fn locking_is_exclusive_released_on_drop_and_keeps_the_inode() {
            let root = TestDirectory::new();
            fs::set_permissions(&root.0, fs::Permissions::from_mode(0o750)).unwrap();
            let directory = root.path("nested/state");
            let lock = lock_directory(&directory).unwrap();
            let path = directory.join("session.lock");
            let inode = fs::metadata(&path).unwrap().ino();
            assert_eq!(fs::metadata(&root.0).unwrap().mode() & 0o777, 0o750);
            assert_eq!(
                fs::metadata(root.path("nested")).unwrap().mode() & 0o777,
                0o700
            );
            assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
            assert!(
                lock_directory(&directory)
                    .err()
                    .unwrap()
                    .contains("Another rstpd-gtk session")
            );
            drop(lock);
            assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
            let second = lock_directory(&directory).unwrap();
            assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
            drop(second);
            assert!(path.exists());
        }

        fn run_lock_probe(directory: &Path, expected: &str) {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "session::tests::linux::lock_probe",
                    "--ignored",
                    "--nocapture",
                ])
                .env("RSTPD_GTK_TEST_LOCK_DIRECTORY", directory)
                .env("RSTPD_GTK_TEST_LOCK_EXPECTED", expected)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "lock subprocess failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        }

        #[test]
        #[ignore = "subprocess fixture exercised by locking_contends_across_processes"]
        fn lock_probe() {
            let path = std::env::var_os("RSTPD_GTK_TEST_LOCK_DIRECTORY").unwrap();
            let expected = std::env::var("RSTPD_GTK_TEST_LOCK_EXPECTED").unwrap();
            let result = lock_directory(Path::new(&path));
            match expected.as_str() {
                "blocked" => assert!(result.err().unwrap().contains("Another rstpd-gtk session")),
                "acquired" => assert!(result.is_ok()),
                _ => panic!("unknown lock probe expectation"),
            }
        }

        #[test]
        fn locking_contends_across_processes() {
            let root = TestDirectory::new();
            let directory = root.path("state");
            let lock = lock_directory(&directory).unwrap();
            let inode = fs::metadata(directory.join("session.lock")).unwrap().ino();
            run_lock_probe(&directory, "blocked");
            drop(lock);
            run_lock_probe(&directory, "acquired");
            assert_eq!(
                fs::metadata(directory.join("session.lock")).unwrap().ino(),
                inode
            );
        }

        #[test]
        fn locking_rejects_symlink_hard_link_and_writable_directory_traps() {
            let root = TestDirectory::new();
            let external = root.path("external");
            fs::write(&external, b"external data").unwrap();
            let linked_directory = root.path("directory-link");
            symlink(".", &linked_directory).unwrap();
            assert!(lock_directory(&linked_directory).is_err());
            assert!(!root.path("session.lock").exists());

            let directory = root.path("state");
            fs::create_dir(&directory).unwrap();
            let path = directory.join("session.lock");
            symlink("../external", &path).unwrap();
            assert!(lock_directory(&directory).is_err());
            assert_eq!(fs::read(&external).unwrap(), b"external data");
            fs::remove_file(&path).unwrap();
            fs::hard_link(&external, &path).unwrap();
            assert!(lock_directory(&directory).is_err());
            assert_eq!(fs::read(&external).unwrap(), b"external data");
            fs::remove_file(&path).unwrap();

            fs::set_permissions(&directory, fs::Permissions::from_mode(0o770)).unwrap();
            assert!(lock_directory(&directory).is_err());
            assert!(!path.exists());
            assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o770);
        }

        #[test]
        fn locking_does_not_truncate_an_existing_lock_file() {
            let root = TestDirectory::new();
            let path = root.path("session.lock");
            fs::write(&path, b"stable lock inode").unwrap();
            let before = fs::metadata(&path).unwrap();
            let lock = lock_directory(&root.0).unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"stable lock inode");
            drop(lock);
            let after = fs::metadata(&path).unwrap();
            assert_eq!(after.ino(), before.ino());
            assert_eq!(after.mode(), before.mode());
            assert_eq!(fs::read(&path).unwrap(), b"stable lock inode");
        }

        #[test]
        fn bounded_reads_and_invalid_recovery_do_not_change_files() {
            let root = TestDirectory::new();
            let path = root.path("data");
            fs::write(&path, b"12345").unwrap();
            assert_eq!(read_bounded(&path, 5).unwrap(), b"12345");
            assert!(read_bounded(&path, 4).is_err());
            fs::write(
                &path,
                br#"{"version":999,"documents":[],"active":0,"theme":"dark"}"#,
            )
            .unwrap();
            let before = fs::read(&path).unwrap();
            assert!(
                load(&path)
                    .unwrap_err()
                    .contains("Unsupported recovery version")
            );
            assert_eq!(fs::read(&path).unwrap(), before);
            let missing = load(&root.path("missing")).unwrap();
            assert_eq!(missing.version, SESSION_VERSION);
            assert_eq!(missing.theme, "system");
        }
    }

    #[test]
    fn fingerprint_is_pinned_across_toolchain_releases() {
        assert_eq!(fingerprint(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fingerprint(b"abc"), 0xe71f_a219_0541_574b);
        assert_ne!(fingerprint(b"abc"), fingerprint(b"abd"));
        assert_ne!(fingerprint(b"ab"), fingerprint(b"abc"));
    }
}

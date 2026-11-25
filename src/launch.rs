use crate::core::Result;
#[cfg(unix)]
use crate::session;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_PENDING_REQUESTS: usize = 32;

#[derive(Debug, Default, PartialEq)]
pub struct Request {
    pub paths: Vec<PathBuf>,
    pub language_paths: Vec<PathBuf>,
    pub api_paths: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    version: u8,
    paths: Vec<Vec<u8>>,
    language_paths: Vec<Vec<u8>>,
    api_paths: Vec<Vec<u8>>,
}

impl Request {
    pub fn new(
        paths: Vec<PathBuf>,
        language_paths: Vec<PathBuf>,
        api_paths: Vec<PathBuf>,
    ) -> Result<Self> {
        let absolute = |paths: Vec<PathBuf>| {
            paths
                .into_iter()
                .map(|path| {
                    if path.as_os_str().is_empty() {
                        return Err("A launch filename cannot be empty.".into());
                    }
                    std::path::absolute(&path)
                        .map_err(|error| format!("{}: {error}", path.display()))
                })
                .collect::<Result<Vec<_>>>()
        };
        Ok(Self {
            paths: absolute(paths)?,
            language_paths: absolute(language_paths)?,
            api_paths: absolute(api_paths)?,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let encode = |paths: &[PathBuf]| {
            paths
                .iter()
                .map(|path| native_bytes(path.as_os_str()))
                .collect()
        };
        let bytes = serde_json::to_vec(&WireRequest {
            version: 1,
            paths: encode(&self.paths),
            language_paths: encode(&self.language_paths),
            api_paths: encode(&self.api_paths),
        })
        .map_err(|error| error.to_string())?;
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("The file launch request exceeds 64 KiB. Open fewer files at once.".into());
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("The file launch request exceeds 64 KiB.".into());
        }
        let wire: WireRequest = serde_json::from_slice(bytes)
            .map_err(|error| format!("Invalid file launch: {error}"))?;
        if wire.version != 1 {
            return Err("Unsupported file launch protocol. Close the older editor first.".into());
        }
        let decode = |paths: Vec<Vec<u8>>| {
            paths
                .into_iter()
                .map(|bytes| native_string(bytes).map(PathBuf::from))
                .collect::<Result<Vec<_>>>()
        };
        let request = Self {
            paths: decode(wire.paths)?,
            language_paths: decode(wire.language_paths)?,
            api_paths: decode(wire.api_paths)?,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<()> {
        let paths = self
            .paths
            .iter()
            .chain(&self.language_paths)
            .chain(&self.api_paths);
        if paths.clone().count() > 256 {
            return Err("A file launch request can contain at most 256 filenames.".into());
        }
        for path in paths {
            if !path.is_absolute() || has_nul(path.as_os_str()) {
                return Err(
                    "Forwarded filenames must be absolute and contain no NUL characters.".into(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
pub fn session_key(directory: &Path) -> Result<String> {
    let directory = directory
        .canonicalize()
        .map_err(|error| format!("Could not resolve the recovery directory: {error}"))?;
    Ok(format!(
        "{:016x}",
        session::fingerprint(&native_bytes(directory.as_os_str()))
    ))
}

#[cfg(unix)]
fn native_bytes(path: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_bytes().to_vec()
}

#[cfg(unix)]
fn native_string(bytes: Vec<u8>) -> Result<OsString> {
    use std::os::unix::ffi::OsStringExt;
    Ok(OsString::from_vec(bytes))
}

#[cfg(unix)]
fn has_nul(path: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    path.as_bytes().contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarding_preserves_absolute_unicode_and_definition_paths() {
        let request = Request::new(
            vec![
                PathBuf::from("file with spaces-\u{65e5}.txt"),
                PathBuf::from("other.txt"),
            ],
            vec![PathBuf::from("language.xml")],
            vec![PathBuf::from("completion.xml")],
        )
        .unwrap();
        assert!(request.paths.iter().all(|path| path.is_absolute()));
        assert_eq!(
            Request::decode(&request.encode().unwrap()).unwrap(),
            request
        );
        assert_eq!(
            Request::decode(&Request::default().encode().unwrap()).unwrap(),
            Request::default()
        );
    }

    #[test]
    fn rejects_invalid_relative_oversized_and_version_mismatched_requests() {
        assert!(Request::decode(b"not json").is_err());
        assert!(Request::decode(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());
        let relative = Request {
            paths: vec![PathBuf::from("relative.txt")],
            ..Default::default()
        };
        assert!(relative.encode().is_err());
        let mut wire = WireRequest {
            version: 2,
            paths: vec![],
            language_paths: vec![],
            api_paths: vec![],
        };
        assert!(Request::decode(&serde_json::to_vec(&wire).unwrap()).is_err());
        wire.version = 1;
        wire.paths.push(native_bytes(OsStr::new("relative.txt")));
        assert!(Request::decode(&serde_json::to_vec(&wire).unwrap()).is_err());
        let absolute = std::env::current_dir().unwrap().join("file.txt");
        wire.paths = vec![native_bytes(absolute.as_os_str()); 257];
        assert!(Request::decode(&serde_json::to_vec(&wire).unwrap()).is_err());
        wire.paths = vec![native_bytes(absolute.as_os_str())];
        wire.paths[0].extend(native_bytes(OsStr::new("\0")));
        assert!(Request::decode(&serde_json::to_vec(&wire).unwrap()).is_err());
    }

    #[test]
    fn request_size_limit_is_enforced_on_both_sides() {
        let mut bytes = Request::default().encode().unwrap();
        bytes.resize(MAX_REQUEST_BYTES, b' ');
        assert!(Request::decode(&bytes).is_ok());
        bytes.push(b' ');
        assert!(Request::decode(&bytes).is_err());
        let request = Request {
            paths: vec![
                std::env::current_dir()
                    .unwrap()
                    .join("a".repeat(MAX_REQUEST_BYTES)),
            ],
            ..Default::default()
        };
        assert!(request.encode().is_err());
    }

    #[test]
    fn directory_aliases_identify_the_same_workspace() {
        let directory = std::env::current_dir().unwrap();
        assert_eq!(
            session_key(&directory).unwrap(),
            session_key(&directory.join(".")).unwrap()
        );
        assert_ne!(
            session_key(&directory).unwrap(),
            session_key(directory.parent().unwrap()).unwrap()
        );
    }
}

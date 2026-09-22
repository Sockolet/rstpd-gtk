use rstpd::{
    core::{Encoding, Eol},
    session::{self, DocumentSnapshot, Session},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime},
};

struct Process(Child);

struct DirectoryCleanup(PathBuf);

impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!(
                "Could not remove test directory {}: {error}",
                self.0.display()
            );
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        match self.0.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(error) = self.0.kill() {
                    eprintln!("Could not stop smoke-test process {}: {error}", self.0.id());
                }
                if let Err(error) = self.0.wait() {
                    eprintln!("Could not reap smoke-test process: {error}");
                }
            }
            Err(error) => eprintln!("Could not inspect smoke-test process: {error}"),
        }
    }
}

fn wait_for(process: &mut Process, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "The editor exited before writing recovery"
        );
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "The running editor did not write recovery"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn cli_crash_recovery_keeps_unsaved_text_and_does_not_replace_it_with_disk_changes() {
    let root = std::env::temp_dir().join(format!(
        "rstpd-gtk-process-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let _cleanup = DirectoryCleanup(root.clone());
    let log = root.join("stderr.log");
    let fresh_state = root.join("fresh");
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_rstpd"))
            .env("XDG_STATE_HOME", &fresh_state)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(&log).unwrap())
            .spawn()
            .unwrap(),
    );
    let fresh_recovery = fresh_state.join("rstpd-gtk/session.json");
    wait_for(&mut process, || fresh_recovery.exists());
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    drop(process);
    let fresh = session::load(&fresh_recovery).unwrap();
    assert_eq!(fresh.documents.len(), 1);
    assert!(fresh.documents[0].path.is_none());
    assert!(fresh.documents[0].text.is_empty());
    assert_eq!(fresh.documents[0].eol, Eol::Lf);
    let directory = root.join("session");
    fs::create_dir(&directory).unwrap();
    let recovery = directory.join("session.json");
    session::save(
        &recovery,
        &Session {
            version: session::SESSION_VERSION,
            documents: vec![DocumentSnapshot {
                id: 1,
                title: "Unsaved".into(),
                path: None,
                text: "unsaved \u{1f680}\n".into(),
                encoding: Encoding::Utf8,
                eol: Eol::Lf,
                language: "Plain text".into(),
                dirty: true,
                disk_hash: None,
                caret: 2,
                pinned: false,
            }],
            theme: "system".into(),
            ..Session::default()
        },
    )
    .unwrap();
    let source = root.join("example.rs");
    fs::write(&source, "fn original() {}\n").unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_rstpd"))
            .arg("--session-dir")
            .arg(&directory)
            .arg("--import-language")
            .arg(fixtures.join("custom-language.xml"))
            .arg("--completion-api")
            .arg(fixtures.join("completion-api.xml"))
            .arg(&source)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::OpenOptions::new().append(true).open(&log).unwrap())
            .spawn()
            .unwrap(),
    );
    wait_for(&mut process, || {
        let saved = session::load(&recovery).unwrap();
        saved.documents.len() == 2
            && saved
                .completion_api
                .iter()
                .any(|api| api.name == "lookup_record")
    });
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    drop(process);
    let saved = session::load(&recovery).unwrap();
    assert_eq!(saved.documents[0].text, "unsaved \u{1f680}\n");
    assert!(saved.documents[0].dirty);
    assert_eq!(saved.documents[1].text, "fn original() {}\n");
    assert_eq!(saved.custom_languages[0].name, "Example data language");
    let modified = fs::metadata(&recovery).unwrap().modified().unwrap();
    fs::write(&source, "external version must stay on disk\n").unwrap();
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_rstpd"))
            .arg("--session-dir")
            .arg(&directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::OpenOptions::new().append(true).open(&log).unwrap())
            .spawn()
            .unwrap(),
    );
    wait_for(&mut process, || {
        fs::metadata(&recovery).unwrap().modified().unwrap() > modified
    });
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    drop(process);
    let recovered = session::load(&recovery).unwrap();
    assert_eq!(recovered.documents[0].text, "unsaved \u{1f680}\n");
    assert_eq!(recovered.documents[1].text, "fn original() {}\n");
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "external version must stay on disk\n"
    );
    assert!(
        recovered
            .completion_api
            .iter()
            .any(|api| api.name == "lookup_record")
    );
    let diagnostics = fs::read_to_string(&log).unwrap();
    assert!(!diagnostics.contains("panicked"), "{diagnostics}");
    assert!(!diagnostics.contains("CRITICAL"), "{diagnostics}");
}

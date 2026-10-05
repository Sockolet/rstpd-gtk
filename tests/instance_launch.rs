#![cfg(target_os = "linux")]

use rstpd::{
    core::{Encoding, Eol},
    launch::Request,
    session::{self, DocumentSnapshot, SESSION_VERSION, Session},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "Instance launch did not complete"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn command(directory: &Path, working_directory: &Path, paths: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rstpd"));
    command
        .current_dir(working_directory)
        .arg("--session-dir")
        .arg(directory)
        .args(paths);
    command
}

struct Running(Child);

impl Running {
    fn start(directory: &Path, working_directory: &Path, paths: &[&str]) -> Self {
        let mut app = Self(
            command(directory, working_directory, paths)
                .spawn()
                .unwrap(),
        );
        rstpd::instance::forward(directory, &Request::default()).unwrap();
        assert!(
            app.0.try_wait().unwrap().is_none(),
            "Editor exited during startup"
        );
        app
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn forward(directory: &Path, working_directory: &Path, paths: &[&str]) {
    finish_launch(command(directory, working_directory, paths));
}

fn finish_launch(mut command: Command) {
    let mut launch = Running(command.spawn().unwrap());
    let mut status = None;
    wait(|| {
        status = launch.0.try_wait().unwrap();
        status.is_some()
    });
    assert!(status.unwrap().success(), "Forwarding process failed");
}

struct DirectoryCleanup(PathBuf);

impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn open_with_reuses_the_matching_instance_and_preserves_unsaved_documents() {
    let directory = std::env::temp_dir().join(format!(
        "rstpd-instance-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let _cleanup = DirectoryCleanup(directory.clone());
    let state = directory.join("state");
    let other_state = directory.join("other-state");
    fs::create_dir(&state).unwrap();
    fs::create_dir(&other_state).unwrap();
    let first = directory.join("first.txt");
    fs::write(&first, "original\n").unwrap();
    fs::write(directory.join("file with spaces-\u{65e5}.txt"), "unicode\n").unwrap();
    fs::write(directory.join("third.txt"), "third\n").unwrap();
    session::save(
        &state.join("session.json"),
        &Session {
            version: SESSION_VERSION,
            theme: "system".into(),
            documents: vec![DocumentSnapshot {
                id: 1,
                title: "first.txt".into(),
                path: Some(first.clone()),
                text: "original\nX".into(),
                encoding: Encoding::Utf8,
                eol: Eol::Lf,
                language: "Normal Text".into(),
                dirty: true,
                disk_hash: Some(session::fingerprint(b"original\n")),
                caret: 10,
                pinned: false,
            }],
            ..Session::default()
        },
    )
    .unwrap();
    let app = Running::start(&state, &directory, &["first.txt"]);
    let other = Running::start(&other_state, &directory, &["third.txt"]);
    assert!(session::try_lock_directory(&state).unwrap().is_none());
    forward(&state.join("."), &directory, &["first.txt"]);
    forward(
        &state,
        &directory,
        &["file with spaces-\u{65e5}.txt", "third.txt"],
    );
    wait(|| {
        session::load(&state.join("session.json")).is_ok_and(|saved| saved.documents.len() == 3)
    });
    forward(&state, &directory, &[]);
    std::thread::sleep(Duration::from_millis(300));
    let saved = session::load(&state.join("session.json")).unwrap();
    assert_eq!(saved.documents.len(), 3);
    assert!(
        saved
            .documents
            .iter()
            .any(|doc| doc.text == "original\nX" && doc.dirty)
    );
    assert!(saved.documents.iter().any(|doc| doc.text == "unicode\n"));
    wait(|| {
        session::load(&other_state.join("session.json"))
            .is_ok_and(|saved| saved.documents.len() == 1)
    });
    let separate = session::load(&other_state.join("session.json")).unwrap();
    assert_eq!(separate.documents[0].text, "third\n");
    assert_eq!(fs::read_to_string(first).unwrap(), "original\n");
    drop(other);
    drop(app);

    let state_root = directory.join("xdg-state");
    let default_state = state_root.join("rstpd-gtk");
    let default_command = |paths: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rstpd"));
        command
            .current_dir(&directory)
            .env("XDG_STATE_HOME", &state_root)
            .args(paths);
        command
    };
    let default_app = Running(default_command(&["first.txt"]).spawn().unwrap());
    wait(|| default_state.join("session.lock").exists());
    rstpd::instance::forward(&default_state, &Request::default()).unwrap();
    finish_launch(default_command(&["file with spaces-\u{65e5}.txt"]));
    wait(|| {
        session::load(&default_state.join("session.json"))
            .is_ok_and(|saved| saved.documents.len() == 2)
    });
    let default_session = session::load(&default_state.join("session.json")).unwrap();
    assert_eq!(
        default_session.documents.len(),
        2,
        "Open with without --session-dir must reuse the default workspace"
    );
    drop(default_app);
}

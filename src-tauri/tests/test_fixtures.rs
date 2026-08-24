mod support;

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::Stdio,
};

use support::{TemporaryAppData, TemporaryGitRemote, deterministic_child};

#[test]
fn temporary_application_data_has_an_isolated_database_path() {
    let app_data = TemporaryAppData::new();
    let root = app_data.path().to_path_buf();

    assert!(root.is_dir());
    assert_eq!(app_data.database_path(), root.join("piu.sqlite3"));

    drop(app_data);
    assert!(!root.exists());
}

#[test]
fn deterministic_child_has_a_line_protocol_and_clean_exit() {
    let mut child = deterministic_child()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn deterministic child");
    let stdout = child.stdout.take().expect("child stdout");
    let mut lines = BufReader::new(stdout).lines();

    assert_eq!(lines.next().unwrap().unwrap(), "ready");
    writeln!(child.stdin.as_mut().unwrap(), "hello").unwrap();
    assert_eq!(lines.next().unwrap().unwrap(), "echo:hello");
    writeln!(child.stdin.as_mut().unwrap(), "exit").unwrap();
    assert_eq!(lines.next().unwrap().unwrap(), "bye");
    assert!(child.wait().unwrap().success());
}

#[test]
fn temporary_git_remote_accepts_a_main_branch() {
    let repositories = TemporaryGitRemote::new();
    fs::write(repositories.working_path().join("README.md"), "fixture\n").unwrap();
    repositories.git(["add", "README.md"]);
    repositories.git(["commit", "-m", "fixture"]);
    repositories.git(["push", "-u", "origin", "main"]);

    let head = repositories.bare_git(["rev-parse", "refs/heads/main"]);
    assert_eq!(head.trim().len(), 40);
}

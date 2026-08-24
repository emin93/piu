use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use piu_lib::git_process::{GitProcess, GitProcessError};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}"))
}

fn process_exists(pid_path: &Path) -> bool {
    let pid = fs::read_to_string(pid_path).expect("fixture should record its process id");
    Command::new("/bin/kill")
        .args(["-0", pid.trim()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("kill should inspect the fixture process")
        .success()
}

#[test]
fn drains_output_larger_than_pipe_capacity_while_supervising_git() {
    let fixture = fixture("verbose-git.zsh");
    let selected = tempfile::tempdir().expect("selected directory should exist");
    let git = GitProcess::with_executable(fixture);

    let discovered = git
        .discover_worktree(selected.path())
        .expect("large stderr output should not deadlock the child");

    assert_eq!(discovered, selected.path());
}

#[test]
fn timeout_terminates_the_entire_git_process_group_and_returns_promptly() {
    let selected = tempfile::tempdir().expect("selected directory should exist");
    let git = GitProcess::with_executable_and_policy(
        fixture("hanging-git.zsh"),
        Duration::from_millis(500),
        64 * 1024,
    );
    let started = Instant::now();

    let error = git
        .discover_worktree(selected.path())
        .expect_err("hung Git should time out");

    assert!(matches!(error, GitProcessError::TimedOut));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "timeout must include descendant termination and pipe cleanup"
    );
    assert!(!process_exists(&selected.path().join("git-parent.pid")));
    assert!(!process_exists(&selected.path().join("git-child.pid")));
}

#[test]
fn rejects_git_output_beyond_the_fixed_capture_budget() {
    let selected = tempfile::tempdir().expect("selected directory should exist");
    let git = GitProcess::with_executable_and_policy(
        fixture("overflow-git.zsh"),
        Duration::from_secs(2),
        1024,
    );

    let error = git
        .discover_worktree(selected.path())
        .expect_err("oversized Git output should be rejected");

    assert!(matches!(error, GitProcessError::OutputLimitExceeded));
}

#[test]
fn bundled_runtime_uses_only_its_fixed_git_paths() {
    let runtime = tempfile::tempdir().expect("runtime directory should exist");
    let bin = runtime.path().join("bin");
    let exec_path = runtime.path().join("libexec/git-core");
    let template_dir = runtime.path().join("share/git-core/templates");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&exec_path).unwrap();
    fs::create_dir_all(&template_dir).unwrap();
    let executable = bin.join("git");
    fs::write(
        &executable,
        r#"#!/bin/zsh
print -r -- "$PATH" > "$2/observed-path"
print -r -- "$GIT_EXEC_PATH" > "$2/observed-exec-path"
print -r -- "$GIT_TEMPLATE_DIR" > "$2/observed-template-dir"
print -r -- "$2"
print -r -- "$2/.git"
"#,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let selected = tempfile::tempdir().expect("selected directory should exist");
    let git = GitProcess::from_bundled_runtime(runtime.path());

    let paths = git.inspect_worktree(selected.path()).unwrap();

    assert_eq!(paths.root, selected.path());
    assert_eq!(paths.git_dir, selected.path().join(".git"));
    assert_eq!(
        fs::read_to_string(selected.path().join("observed-path"))
            .unwrap()
            .trim(),
        format!("{}:/usr/bin:/bin", bin.display())
    );
    assert_eq!(
        fs::read_to_string(selected.path().join("observed-exec-path"))
            .unwrap()
            .trim(),
        exec_path.to_string_lossy()
    );
    assert_eq!(
        fs::read_to_string(selected.path().join("observed-template-dir"))
            .unwrap()
            .trim(),
        template_dir.to_string_lossy()
    );
}

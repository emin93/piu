use std::path::PathBuf;

use piu_lib::git_process::GitProcess;

#[test]
fn drains_output_larger_than_pipe_capacity_while_supervising_git() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/verbose-git.zsh");
    let selected = tempfile::tempdir().expect("selected directory should exist");
    let git = GitProcess::with_executable(fixture);

    let discovered = git
        .discover_worktree(selected.path())
        .expect("large stderr output should not deadlock the child");

    assert_eq!(discovered, selected.path());
}

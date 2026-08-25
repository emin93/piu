use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tempfile::TempDir;

fn isolated_git_command() -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0");
    command
}

pub struct TemporaryAppData {
    root: TempDir,
}

impl TemporaryAppData {
    pub fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("create temporary application data"),
        }
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn database_path(&self) -> PathBuf {
        self.path().join("piu.sqlite3")
    }
}

pub fn deterministic_child() -> Command {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/deterministic-child.zsh");
    let mut command = Command::new("/bin/zsh");
    command.arg(fixture);
    command
}

pub struct TemporaryGitRemote {
    _root: TempDir,
    working: PathBuf,
    remote: PathBuf,
}

impl TemporaryGitRemote {
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("create temporary Git fixture");
        let working = root.path().join("working");
        let remote = root.path().join("remote.git");

        run(
            isolated_git_command()
                .args(["init", "--bare", "--initial-branch=main"])
                .arg(&remote),
            "initialize bare remote",
        );
        run(
            isolated_git_command()
                .args(["init", "--initial-branch=main"])
                .arg(&working),
            "initialize working repository",
        );
        let fixture = Self {
            _root: root,
            working,
            remote,
        };
        fixture.git(["config", "user.name", "Più Test"]);
        fixture.git(["config", "user.email", "piu-test@example.invalid"]);
        fixture.git(["config", "commit.gpgSign", "false"]);
        run(
            fixture
                .git_command()
                .args(["remote", "add", "origin"])
                .arg(&fixture.remote),
            "add fixture remote",
        );
        fixture
    }

    pub fn working_path(&self) -> &Path {
        &self.working
    }

    pub fn git<I, S>(&self, arguments: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run(
            self.git_command().args(arguments),
            "run Git fixture command",
        )
    }

    pub fn bare_git<I, S>(&self, arguments: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = isolated_git_command();
        command.arg("--git-dir").arg(&self.remote).args(arguments);
        run(&mut command, "run bare Git fixture command")
    }

    fn git_command(&self) -> Command {
        let mut command = isolated_git_command();
        command.arg("-C").arg(&self.working);
        command
    }
}

fn run(command: &mut Command, description: &str) -> String {
    let Output {
        status,
        stdout,
        stderr,
    } = command
        .output()
        .unwrap_or_else(|error| panic!("{description}: {error}"));
    assert!(
        status.success(),
        "{description} failed: {}",
        String::from_utf8_lossy(&stderr)
    );
    String::from_utf8(stdout).expect("fixture command output is UTF-8")
}

use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use thiserror::Error;

const INSPECTION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum GitProcessError {
    #[error("could not start Git: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("could not supervise Git: {0}")]
    Supervision(#[source] std::io::Error),
    #[error("Git inspection timed out")]
    TimedOut,
    #[error("Git exited unsuccessfully ({status:?}): {stderr}")]
    Failed { status: Option<i32>, stderr: String },
    #[error("Git returned a path that was not valid UTF-8")]
    InvalidOutput,
}

pub struct GitProcess {
    executable: PathBuf,
}

impl Default for GitProcess {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("git"),
        }
    }
}

impl GitProcess {
    pub fn with_executable(executable: PathBuf) -> Self {
        Self { executable }
    }

    pub fn discover_worktree(&self, selected_path: &Path) -> Result<PathBuf, GitProcessError> {
        let output = self.run([
            OsStr::new("-C"),
            selected_path.as_os_str(),
            OsStr::new("rev-parse"),
            OsStr::new("--show-toplevel"),
        ])?;
        let path = String::from_utf8(output).map_err(|_| GitProcessError::InvalidOutput)?;
        Ok(PathBuf::from(path.trim_end()))
    }

    fn run<const N: usize>(&self, args: [&OsStr; N]) -> Result<Vec<u8>, GitProcessError> {
        let mut child = Command::new(&self.executable)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(GitProcessError::Spawn)?;
        let stdout_reader = drain_pipe(child.stdout.take())?;
        let stderr_reader = drain_pipe(child.stderr.take())?;
        let deadline = Instant::now() + INSPECTION_TIMEOUT;
        let status = loop {
            match child.try_wait().map_err(GitProcessError::Supervision)? {
                Some(status) => break status,
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                None => {
                    child.kill().map_err(GitProcessError::Supervision)?;
                    child.wait().map_err(GitProcessError::Supervision)?;
                    finish_drain(stdout_reader)?;
                    finish_drain(stderr_reader)?;
                    return Err(GitProcessError::TimedOut);
                }
            }
        };
        let stdout = finish_drain(stdout_reader)?;
        let stderr = finish_drain(stderr_reader)?;
        successful_output(status, stdout, stderr)
    }
}

fn drain_pipe(
    pipe: Option<impl Read + Send + 'static>,
) -> Result<thread::JoinHandle<std::io::Result<Vec<u8>>>, GitProcessError> {
    let mut pipe = pipe.ok_or_else(|| {
        GitProcessError::Supervision(std::io::Error::other("child pipe was unavailable"))
    })?;
    Ok(thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)?;
        Ok(bytes)
    }))
}

fn finish_drain(
    reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<Vec<u8>, GitProcessError> {
    reader
        .join()
        .map_err(|_| GitProcessError::Supervision(std::io::Error::other("pipe reader panicked")))?
        .map_err(GitProcessError::Supervision)
}

fn successful_output(
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
) -> Result<Vec<u8>, GitProcessError> {
    if status.success() {
        Ok(stdout)
    } else {
        Err(GitProcessError::Failed {
            status: status.code(),
            stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
        })
    }
}

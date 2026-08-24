use std::{
    env,
    ffi::OsStr,
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use rustix::process::{Pid, Signal, kill_process_group};
use thiserror::Error;

const INSPECTION_TIMEOUT: Duration = Duration::from_secs(5);
const NETWORK_OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
const WORKTREE_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CAPTURED_OUTPUT_BYTES: usize = 256 * 1024;

#[derive(Debug, Error)]
pub enum GitProcessError {
    #[error("could not start Git: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("could not supervise Git: {0}")]
    Supervision(#[source] std::io::Error),
    #[error("Git inspection timed out")]
    TimedOut,
    #[error("Git produced more diagnostic output than Più accepts")]
    OutputLimitExceeded,
    #[error("Git exited unsuccessfully ({status:?}): {stderr}")]
    Failed { status: Option<i32>, stderr: String },
    #[error("Git returned a path that was not valid UTF-8")]
    InvalidOutput,
}

#[derive(Clone)]
pub struct GitProcess {
    executable: PathBuf,
    runtime_environment: Option<GitRuntimeEnvironment>,
    timeout: Duration,
    max_output_bytes: usize,
}

#[derive(Clone)]
struct GitRuntimeEnvironment {
    exec_path: PathBuf,
    template_dir: PathBuf,
    path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitWorktreePaths {
    pub root: PathBuf,
    pub git_dir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedWorktreeIdentity {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_git_dir: PathBuf,
    pub head: String,
}

impl GitProcess {
    pub fn from_bundled_runtime(runtime_root: &Path) -> Self {
        let bin = runtime_root.join("bin");
        Self {
            executable: bin.join("git"),
            runtime_environment: Some(GitRuntimeEnvironment {
                exec_path: runtime_root.join("libexec/git-core"),
                template_dir: runtime_root.join("share/git-core/templates"),
                path: PathBuf::from(
                    env::join_paths([bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")])
                        .expect("fixed bundled Git paths must form a valid PATH"),
                ),
            }),
            timeout: INSPECTION_TIMEOUT,
            max_output_bytes: MAX_CAPTURED_OUTPUT_BYTES,
        }
    }

    pub fn with_executable(executable: PathBuf) -> Self {
        Self {
            executable,
            runtime_environment: None,
            timeout: INSPECTION_TIMEOUT,
            max_output_bytes: MAX_CAPTURED_OUTPUT_BYTES,
        }
    }

    pub fn with_executable_and_policy(
        executable: PathBuf,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Self {
        Self {
            executable,
            runtime_environment: None,
            timeout,
            max_output_bytes,
        }
    }

    pub fn discover_worktree(&self, selected_path: &Path) -> Result<PathBuf, GitProcessError> {
        Ok(self.inspect_worktree(selected_path)?.root)
    }

    pub fn inspect_worktree(
        &self,
        selected_path: &Path,
    ) -> Result<GitWorktreePaths, GitProcessError> {
        let output = self.run_with_timeout(
            [
                OsStr::new("-C"),
                selected_path.as_os_str(),
                OsStr::new("rev-parse"),
                OsStr::new("--show-toplevel"),
                OsStr::new("--absolute-git-dir"),
            ],
            self.timeout,
        )?;
        let paths = String::from_utf8(output).map_err(|_| GitProcessError::InvalidOutput)?;
        let mut lines = paths.lines();
        let root = lines.next().filter(|line| !line.is_empty());
        let git_dir = lines.next().filter(|line| !line.is_empty());
        if lines.next().is_some() || root.is_none() || git_dir.is_none() {
            return Err(GitProcessError::InvalidOutput);
        }
        Ok(GitWorktreePaths {
            root: PathBuf::from(root.expect("root was checked")),
            git_dir: PathBuf::from(git_dir.expect("Git directory was checked")),
        })
    }

    pub fn fetch_origin_main(&self, repository: &Path) -> Result<String, GitProcessError> {
        self.run_with_timeout(
            [
                OsStr::new("-C"),
                repository.as_os_str(),
                OsStr::new("fetch"),
                OsStr::new("--no-tags"),
                OsStr::new("--prune"),
                OsStr::new("origin"),
                OsStr::new("+refs/heads/main:refs/remotes/origin/main"),
            ],
            NETWORK_OPERATION_TIMEOUT,
        )?;
        let output = self.run_with_timeout(
            [
                OsStr::new("-C"),
                repository.as_os_str(),
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("refs/remotes/origin/main^{commit}"),
            ],
            self.timeout,
        )?;
        single_line_output(output)
    }

    pub fn add_detached_worktree(
        &self,
        repository: &Path,
        worktree: &Path,
        commit: &str,
    ) -> Result<(), GitProcessError> {
        self.run_with_timeout(
            [
                OsStr::new("-C"),
                repository.as_os_str(),
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("--detach"),
                worktree.as_os_str(),
                OsStr::new(commit),
            ],
            WORKTREE_OPERATION_TIMEOUT,
        )?;
        Ok(())
    }

    pub fn attach_new_branch(&self, worktree: &Path, branch: &str) -> Result<(), GitProcessError> {
        self.run_with_timeout(
            [
                OsStr::new("-C"),
                worktree.as_os_str(),
                OsStr::new("switch"),
                OsStr::new("-c"),
                OsStr::new(branch),
            ],
            WORKTREE_OPERATION_TIMEOUT,
        )?;
        Ok(())
    }

    pub fn current_branch(&self, worktree: &Path) -> Result<Option<String>, GitProcessError> {
        let output = self.run_with_timeout(
            [
                OsStr::new("-C"),
                worktree.as_os_str(),
                OsStr::new("symbolic-ref"),
                OsStr::new("--quiet"),
                OsStr::new("--short"),
                OsStr::new("HEAD"),
            ],
            self.timeout,
        );
        match output {
            Ok(output) => single_line_output(output).map(Some),
            Err(GitProcessError::Failed {
                status: Some(1), ..
            }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn inspect_managed_worktree(
        &self,
        worktree: &Path,
    ) -> Result<ManagedWorktreeIdentity, GitProcessError> {
        let output = self.run_with_timeout(
            [
                OsStr::new("-C"),
                worktree.as_os_str(),
                OsStr::new("rev-parse"),
                OsStr::new("--path-format=absolute"),
                OsStr::new("--show-toplevel"),
                OsStr::new("--git-dir"),
                OsStr::new("--git-common-dir"),
                OsStr::new("--verify"),
                OsStr::new("HEAD^{commit}"),
            ],
            self.timeout,
        )?;
        let text = String::from_utf8(output).map_err(|_| GitProcessError::InvalidOutput)?;
        let mut lines = text.lines();
        let root = lines.next().filter(|line| !line.is_empty());
        let git_dir = lines.next().filter(|line| !line.is_empty());
        let common_git_dir = lines.next().filter(|line| !line.is_empty());
        let head = lines.next().filter(|line| !line.is_empty());
        if lines.next().is_some()
            || root.is_none()
            || git_dir.is_none()
            || common_git_dir.is_none()
            || head.is_none()
        {
            return Err(GitProcessError::InvalidOutput);
        }
        Ok(ManagedWorktreeIdentity {
            root: PathBuf::from(root.expect("root was checked")),
            git_dir: PathBuf::from(git_dir.expect("Git dir was checked")),
            common_git_dir: PathBuf::from(common_git_dir.expect("common Git dir was checked")),
            head: head.expect("HEAD was checked").to_owned(),
        })
    }

    pub fn remove_worktree(
        &self,
        repository: &Path,
        worktree: &Path,
    ) -> Result<(), GitProcessError> {
        self.run_with_timeout(
            [
                OsStr::new("-C"),
                repository.as_os_str(),
                OsStr::new("worktree"),
                OsStr::new("remove"),
                OsStr::new("--force"),
                worktree.as_os_str(),
            ],
            WORKTREE_OPERATION_TIMEOUT,
        )?;
        Ok(())
    }

    pub fn worktree_is_pristine(&self, worktree: &Path) -> Result<bool, GitProcessError> {
        let output = self.run_with_timeout(
            [
                OsStr::new("-C"),
                worktree.as_os_str(),
                OsStr::new("status"),
                OsStr::new("--porcelain=v1"),
                OsStr::new("--untracked-files=all"),
                OsStr::new("--ignored=matching"),
            ],
            self.timeout,
        )?;
        Ok(output.is_empty())
    }

    pub fn prune_worktrees(&self, repository: &Path) -> Result<(), GitProcessError> {
        self.run_with_timeout(
            [
                OsStr::new("-C"),
                repository.as_os_str(),
                OsStr::new("worktree"),
                OsStr::new("prune"),
            ],
            WORKTREE_OPERATION_TIMEOUT,
        )?;
        Ok(())
    }

    pub fn branch_exists(&self, repository: &Path, branch: &str) -> Result<bool, GitProcessError> {
        let reference = format!("refs/heads/{branch}");
        match self.run_with_timeout(
            [
                OsStr::new("-C"),
                repository.as_os_str(),
                OsStr::new("show-ref"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new(&reference),
            ],
            self.timeout,
        ) {
            Ok(_) => Ok(true),
            Err(GitProcessError::Failed {
                status: Some(1), ..
            }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn delete_branch(&self, repository: &Path, branch: &str) -> Result<(), GitProcessError> {
        self.run_with_timeout(
            [
                OsStr::new("-C"),
                repository.as_os_str(),
                OsStr::new("branch"),
                OsStr::new("-D"),
                OsStr::new(branch),
            ],
            WORKTREE_OPERATION_TIMEOUT,
        )?;
        Ok(())
    }

    fn run_with_timeout<const N: usize>(
        &self,
        args: [&OsStr; N],
        timeout: Duration,
    ) -> Result<Vec<u8>, GitProcessError> {
        let mut command = Command::new(&self.executable);
        command
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        if let Some(runtime) = &self.runtime_environment {
            command
                .env("GIT_EXEC_PATH", &runtime.exec_path)
                .env("GIT_TEMPLATE_DIR", &runtime.template_dir)
                .env("PATH", &runtime.path);
        }
        let mut child = command.spawn().map_err(GitProcessError::Spawn)?;
        let output_limited = Arc::new(AtomicBool::new(false));
        let readers = match ChildReaders::start(
            &mut child,
            self.max_output_bytes,
            Arc::clone(&output_limited),
        ) {
            Ok(readers) => readers,
            Err(error) => {
                let _ = terminate_and_reap(&mut child, None);
                return Err(error);
            }
        };
        let deadline = Instant::now() + timeout;
        let status = loop {
            if output_limited.load(Ordering::Acquire) {
                terminate_and_reap(&mut child, Some(readers))?;
                return Err(GitProcessError::OutputLimitExceeded);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    terminate_and_reap(&mut child, Some(readers))?;
                    return Err(GitProcessError::TimedOut);
                }
                Err(error) => {
                    let cleanup = terminate_and_reap(&mut child, Some(readers));
                    return cleanup.and(Err(GitProcessError::Supervision(error)));
                }
            }
        };
        let (stdout, stderr) = readers.finish()?;
        if output_limited.load(Ordering::Acquire) {
            return Err(GitProcessError::OutputLimitExceeded);
        }
        successful_output(status, stdout, stderr)
    }
}

fn single_line_output(output: Vec<u8>) -> Result<String, GitProcessError> {
    let text = String::from_utf8(output).map_err(|_| GitProcessError::InvalidOutput)?;
    let mut lines = text.lines();
    let value = lines.next().filter(|line| !line.is_empty());
    if lines.next().is_some() || value.is_none() {
        return Err(GitProcessError::InvalidOutput);
    }
    Ok(value.expect("single line was checked").to_owned())
}

struct ChildReaders {
    stdout: JoinHandle<std::io::Result<Vec<u8>>>,
    stderr: JoinHandle<std::io::Result<Vec<u8>>>,
}

impl ChildReaders {
    fn start(
        child: &mut Child,
        max_output_bytes: usize,
        output_limited: Arc<AtomicBool>,
    ) -> Result<Self, GitProcessError> {
        let stdout_pipe = child.stdout.take().ok_or_else(missing_child_pipe)?;
        let stderr_pipe = child.stderr.take().ok_or_else(missing_child_pipe)?;
        let stdout = drain_pipe(stdout_pipe, max_output_bytes, Arc::clone(&output_limited));
        let stderr = drain_pipe(stderr_pipe, max_output_bytes, output_limited);
        Ok(Self { stdout, stderr })
    }

    fn finish(self) -> Result<(Vec<u8>, Vec<u8>), GitProcessError> {
        let stdout = finish_drain(self.stdout);
        let stderr = finish_drain(self.stderr);
        Ok((stdout?, stderr?))
    }
}

fn missing_child_pipe() -> GitProcessError {
    GitProcessError::Supervision(std::io::Error::other("child pipe was unavailable"))
}

fn drain_pipe(
    mut pipe: impl Read + Send + 'static,
    max_output_bytes: usize,
    output_limited: Arc<AtomicBool>,
) -> JoinHandle<std::io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(max_output_bytes.min(8 * 1024));
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let read = pipe.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            let remaining = max_output_bytes.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..read.min(remaining)]);
            if read > remaining {
                output_limited.store(true, Ordering::Release);
            }
        }
        Ok(bytes)
    })
}

fn finish_drain(reader: JoinHandle<std::io::Result<Vec<u8>>>) -> Result<Vec<u8>, GitProcessError> {
    reader
        .join()
        .map_err(|_| GitProcessError::Supervision(std::io::Error::other("pipe reader panicked")))?
        .map_err(GitProcessError::Supervision)
}

fn terminate_and_reap(
    child: &mut Child,
    readers: Option<ChildReaders>,
) -> Result<(), GitProcessError> {
    let terminated = child
        .id()
        .try_into()
        .ok()
        .and_then(Pid::from_raw)
        .is_some_and(|pid| kill_process_group(pid, Signal::KILL).is_ok());
    let kill_result = if terminated { Ok(()) } else { child.kill() };
    let wait_result = child.wait();
    let reader_result = readers.map(ChildReaders::finish).transpose();

    kill_result.map_err(GitProcessError::Supervision)?;
    wait_result.map_err(GitProcessError::Supervision)?;
    reader_result?;
    Ok(())
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

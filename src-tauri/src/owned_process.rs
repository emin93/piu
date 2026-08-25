use std::{
    collections::BTreeMap, ffi::OsString, io, os::unix::process::CommandExt, path::Path,
    process::Stdio,
};

use rustix::process::{Pid, Signal, kill_process_group};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

/// A child whose explicit environment, pipes, and fresh process group belong to Più.
pub(crate) struct OwnedPipedProcess {
    pub(crate) child: Child,
    pub(crate) stdin: ChildStdin,
    pub(crate) stdout: ChildStdout,
    pub(crate) stderr: ChildStderr,
    pub(crate) process_group: OwnedProcessGroup,
}

/// The process group rooted at a Più-owned child.
#[derive(Clone, Copy)]
pub(crate) struct OwnedProcessGroup {
    leader: u32,
}

impl OwnedProcessGroup {
    pub(crate) fn force_kill(self) {
        if let Some(process_group) = self.leader.try_into().ok().and_then(Pid::from_raw) {
            let _ = kill_process_group(process_group, Signal::KILL);
        }
    }
}

/// Spawns one child with no inherited environment, three owned pipes, and a fresh process group.
pub(crate) fn spawn_owned_piped_process(
    executable: &Path,
    arguments: &[OsString],
    working_directory: &Path,
    environment: &BTreeMap<OsString, OsString>,
) -> io::Result<OwnedPipedProcess> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(working_directory)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);

    let mut child = command.spawn()?;
    let leader = child
        .id()
        .ok_or_else(|| io::Error::other("child process ID was unavailable"))?;
    let process_group = OwnedProcessGroup { leader };
    let stdin = child.stdin.take().ok_or_else(|| {
        process_group.force_kill();
        io::Error::other("child stdin was unavailable")
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        process_group.force_kill();
        io::Error::other("child stdout was unavailable")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        process_group.force_kill();
        io::Error::other("child stderr was unavailable")
    })?;

    Ok(OwnedPipedProcess {
        child,
        stdin,
        stdout,
        stderr,
        process_group,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, ffi::OsString, path::Path, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        time::timeout,
    };

    use super::spawn_owned_piped_process;

    #[tokio::test]
    async fn spawn_owns_clean_environment_and_all_standard_pipes() {
        let environment = BTreeMap::from([(
            OsString::from("PIU_OWNED_PROCESS_TEST"),
            OsString::from("explicit"),
        )]);
        let arguments = [
            OsString::from("-c"),
            OsString::from(
                "IFS= read -r line; test -z \"${HOME+x}\"; printf '%s:%s' \"$PIU_OWNED_PROCESS_TEST\" \"$line\"; printf 'diagnostic' >&2",
            ),
        ];
        let mut process = spawn_owned_piped_process(
            Path::new("/bin/sh"),
            &arguments,
            Path::new("/tmp"),
            &environment,
        )
        .expect("owned child should start");

        process
            .stdin
            .write_all(b"input\n")
            .await
            .expect("stdin should be writable");
        process.stdin.shutdown().await.expect("stdin should close");
        let mut stdout = String::new();
        process
            .stdout
            .read_to_string(&mut stdout)
            .await
            .expect("stdout should be readable");
        let mut stderr = String::new();
        process
            .stderr
            .read_to_string(&mut stderr)
            .await
            .expect("stderr should be readable");
        let status = process.child.wait().await.expect("child should be reaped");

        assert!(status.success());
        assert_eq!(stdout, "explicit:input");
        assert_eq!(stderr, "diagnostic");
    }

    #[tokio::test]
    async fn process_group_handle_force_kills_the_owned_child() {
        let arguments = [OsString::from("-c"), OsString::from("sleep 30")];
        let mut process = spawn_owned_piped_process(
            Path::new("/bin/sh"),
            &arguments,
            Path::new("/tmp"),
            &BTreeMap::new(),
        )
        .expect("owned child should start");

        process.process_group.force_kill();
        let status = timeout(Duration::from_secs(2), process.child.wait())
            .await
            .expect("forced child should exit promptly")
            .expect("forced child should be reaped");

        assert!(!status.success());
    }
}

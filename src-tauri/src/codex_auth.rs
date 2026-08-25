use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Child,
    sync::{Mutex as AsyncMutex, broadcast, mpsc, watch},
    time::timeout,
};
use ts_rs::TS;

use crate::owned_process::{OwnedProcessGroup, spawn_owned_piped_process};

const MINIMAL_PATH: &str = "/usr/bin:/bin";

/// The complete, explicit invocation of the short-lived Codex authentication helper.
#[derive(Clone, Debug)]
pub struct CodexAuthProcessSpec {
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
}

impl CodexAuthProcessSpec {
    pub fn from_bundled_runtime(
        resource_directory: &Path,
        application_data_directory: &Path,
        real_home_directory: &Path,
    ) -> Self {
        let runtime = resource_directory.join("agent-runtime");
        let pi = runtime.join("pi");
        let launcher = pi.join("launcher/auth-launcher.mjs");
        let credential_locks = application_data_directory.join("credential-locks");
        let mut environment = BTreeMap::new();
        environment.insert(
            OsString::from("HOME"),
            real_home_directory.as_os_str().to_owned(),
        );
        environment.insert(OsString::from("PATH"), OsString::from(MINIMAL_PATH));
        environment.insert(OsString::from("PI_SKIP_VERSION_CHECK"), OsString::from("1"));
        environment.insert(OsString::from("PI_TELEMETRY"), OsString::from("0"));
        Self {
            executable: runtime.join("node/bin/node"),
            arguments: vec![
                launcher.into_os_string(),
                OsString::from("--credential-lock-dir"),
                credential_locks.into_os_string(),
            ],
            working_directory: pi,
            environment,
        }
    }
}

/// Fixed host limits. None of these values are user settings.
#[derive(Clone, Debug)]
pub struct CodexAuthPolicy {
    pub operation_timeout: Duration,
    pub graceful_shutdown_timeout: Duration,
    pub maximum_record_bytes: usize,
    pub maximum_response_bytes: usize,
    pub write_queue_capacity: usize,
    pub update_queue_capacity: usize,
}

impl Default for CodexAuthPolicy {
    fn default() -> Self {
        Self {
            operation_timeout: Duration::from_secs(15 * 60),
            graceful_shutdown_timeout: Duration::from_secs(2),
            maximum_record_bytes: 64 * 1024,
            maximum_response_bytes: 16 * 1024,
            write_queue_capacity: 8,
            update_queue_capacity: 64,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct CodexAuthLink {
    pub url: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct CodexAuthOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type")]
#[ts(export, export_to = "../../src/generated/")]
pub enum CodexAuthEvent {
    #[serde(rename = "info")]
    Info {
        message: String,
        #[serde(default)]
        links: Vec<CodexAuthLink>,
    },
    #[serde(rename = "auth_url")]
    AuthUrl {
        url: String,
        instructions: Option<String>,
    },
    #[serde(rename = "device_code")]
    DeviceCode {
        #[serde(rename = "userCode")]
        user_code: String,
        #[serde(rename = "verificationUri")]
        verification_uri: String,
        #[serde(rename = "intervalSeconds")]
        interval_seconds: Option<f64>,
        #[serde(rename = "expiresInSeconds")]
        expires_in_seconds: Option<f64>,
    },
    #[serde(rename = "progress")]
    Progress { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type")]
#[ts(export, export_to = "../../src/generated/")]
pub enum CodexAuthPrompt {
    #[serde(rename = "select")]
    Select {
        message: String,
        options: Vec<CodexAuthOption>,
    },
    #[serde(rename = "text")]
    Text {
        message: String,
        placeholder: Option<String>,
    },
    #[serde(rename = "secret")]
    Secret {
        message: String,
        placeholder: Option<String>,
    },
    #[serde(rename = "manual_code")]
    ManualCode {
        message: String,
        placeholder: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "type")]
#[ts(export, export_to = "../../src/generated/")]
pub enum CodexAuthUpdate {
    #[serde(rename = "auth_event")]
    Event { event: CodexAuthEvent },
    #[serde(rename = "auth_prompt")]
    Prompt { id: String, prompt: CodexAuthPrompt },
    #[serde(rename = "auth_prompt_cancelled")]
    PromptCancelled { id: String },
    #[serde(rename = "auth_complete")]
    Complete,
    #[serde(rename = "auth_cancelled")]
    Cancelled,
    #[serde(rename = "auth_failed")]
    Failed { code: String, message: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum CodexAuthStatus {
    #[default]
    SignedOut,
    SigningIn,
    WaitingForInput {
        #[serde(rename = "promptId")]
        prompt_id: String,
    },
    Cancelling,
    SignedIn,
    Cancelled,
    Failed {
        code: CodexAuthFailureCode,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum CodexAuthFailureCode {
    SignInFailed,
    SignInTimedOut,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CodexAuthError {
    #[error("authentication helper paths must be absolute")]
    NonAbsoluteProcessPath,
    #[error("authentication helper requires the user's real HOME")]
    MissingHome,
    #[error("authentication helper limits must be greater than zero")]
    InvalidPolicy,
    #[error("authentication is already running")]
    AlreadyRunning,
    #[error("authentication is not running")]
    NotRunning,
    #[error("authentication prompt is no longer waiting for an answer")]
    PromptNotPending,
    #[error("authentication prompt response is invalid")]
    InvalidPromptResponse,
    #[error("could not start authentication")]
    Spawn,
    #[error("authentication helper protocol failed")]
    Protocol,
    #[error("authentication helper I/O failed")]
    Io,
    #[error("authentication timed out")]
    TimedOut,
    #[error("authentication update consumer fell behind by {missed} records")]
    UpdateBackpressure { missed: u64 },
}

pub struct CodexAuthUpdates {
    receiver: broadcast::Receiver<CodexAuthUpdate>,
}

impl CodexAuthUpdates {
    pub async fn recv(&mut self) -> Result<CodexAuthUpdate, CodexAuthError> {
        self.receiver.recv().await.map_err(|error| match error {
            broadcast::error::RecvError::Closed => CodexAuthError::NotRunning,
            broadcast::error::RecvError::Lagged(missed) => {
                CodexAuthError::UpdateBackpressure { missed }
            }
        })
    }
}

/// Owns the complete graphical authentication operation and at most one helper process.
#[derive(Clone)]
pub struct CodexAuthManager {
    inner: Arc<Inner>,
}

impl CodexAuthManager {
    pub fn new(
        process: CodexAuthProcessSpec,
        policy: CodexAuthPolicy,
    ) -> Result<Self, CodexAuthError> {
        validate(&process, &policy)?;
        let (updates, _) = broadcast::channel(policy.update_queue_capacity);
        let (activity, _) = watch::channel(false);
        Ok(Self {
            inner: Arc::new(Inner {
                process,
                policy,
                launch_gate: AsyncMutex::new(()),
                state: Mutex::new(State::default()),
                updates,
                activity,
            }),
        })
    }

    pub fn from_bundled_runtime(
        resource_directory: &Path,
        application_data_directory: &Path,
        real_home_directory: &Path,
    ) -> Result<Self, CodexAuthError> {
        Self::new(
            CodexAuthProcessSpec::from_bundled_runtime(
                resource_directory,
                application_data_directory,
                real_home_directory,
            ),
            CodexAuthPolicy::default(),
        )
    }

    pub fn status(&self) -> CodexAuthStatus {
        self.inner
            .state
            .lock()
            .expect("authentication state lock was poisoned")
            .status
            .clone()
    }

    pub fn subscribe(&self) -> CodexAuthUpdates {
        CodexAuthUpdates {
            receiver: self.inner.updates.subscribe(),
        }
    }

    pub async fn start(&self) -> Result<(), CodexAuthError> {
        let _launch = self.inner.launch_gate.lock().await;
        {
            let state = self
                .inner
                .state
                .lock()
                .expect("authentication state lock was poisoned");
            if state.active.is_some() {
                return Err(CodexAuthError::AlreadyRunning);
            }
        }

        create_credential_lock_directory(&self.inner.process.arguments).await?;
        let process = spawn_owned_piped_process(
            &self.inner.process.executable,
            &self.inner.process.arguments,
            &self.inner.process.working_directory,
            &self.inner.process.environment,
        )
        .map_err(|_| CodexAuthError::Spawn)?;
        let process_group = process.process_group;
        let generation = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("authentication state lock was poisoned");
            if state.active.is_some() {
                process_group.force_kill();
                return Err(CodexAuthError::AlreadyRunning);
            }
            state.next_generation += 1;
            let generation = state.next_generation;
            let (writer, writer_receiver) = mpsc::channel(self.inner.policy.write_queue_capacity);
            let (control, control_receiver) = mpsc::channel(2);
            state.active = Some(Active {
                generation,
                process_group,
                writer,
                control,
                pending_prompts: BTreeSet::new(),
                answered_prompts: BTreeSet::new(),
                cancel_requested: false,
                terminal: None,
            });
            state.status = CodexAuthStatus::SigningIn;
            self.inner.activity.send_replace(true);
            spawn_tasks(
                Arc::downgrade(&self.inner),
                generation,
                process.child,
                process.stdin,
                process.stdout,
                process.stderr,
                process_group,
                writer_receiver,
                control_receiver,
            );
            generation
        };

        let weak = Arc::downgrade(&self.inner);
        let operation_timeout = self.inner.policy.operation_timeout;
        tokio::spawn(async move {
            tokio::time::sleep(operation_timeout).await;
            if let Some(inner) = weak.upgrade() {
                inner.fail(generation, CodexAuthError::TimedOut);
            }
        });
        Ok(())
    }

    pub async fn answer(&self, prompt_id: &str, value: &str) -> Result<(), CodexAuthError> {
        if prompt_id.is_empty()
            || value.is_empty()
            || value.len() > self.inner.policy.maximum_response_bytes
        {
            return Err(CodexAuthError::InvalidPromptResponse);
        }
        let (generation, writer) = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("authentication state lock was poisoned");
            let writer = {
                let active = state.active.as_mut().ok_or(CodexAuthError::NotRunning)?;
                if !active.pending_prompts.remove(prompt_id) {
                    return Err(CodexAuthError::PromptNotPending);
                }
                active.answered_prompts.insert(prompt_id.to_owned());
                active.writer.clone()
            };
            state.status = CodexAuthStatus::SigningIn;
            let generation = state
                .active
                .as_ref()
                .expect("active authentication disappeared")
                .generation;
            (generation, writer)
        };
        let mut frame = serde_json::to_vec(&PromptResponse {
            kind: "auth_prompt_response",
            id: prompt_id,
            value,
        })
        .map_err(|_| CodexAuthError::Io)?;
        frame.push(b'\n');
        if writer.send(frame).await.is_err() {
            self.inner.fail(generation, CodexAuthError::Io);
            return Err(CodexAuthError::Io);
        }
        Ok(())
    }

    pub async fn cancel(&self) -> Result<(), CodexAuthError> {
        let (generation, writer) = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("authentication state lock was poisoned");
            let active = state.active.as_mut().ok_or(CodexAuthError::NotRunning)?;
            if active.terminal.is_some() {
                return Err(CodexAuthError::NotRunning);
            }
            active.cancel_requested = true;
            let result = (active.generation, active.writer.clone());
            state.status = CodexAuthStatus::Cancelling;
            result
        };
        if writer
            .send(b"{\"type\":\"auth_cancel\"}\n".to_vec())
            .await
            .is_err()
        {
            self.inner.fail(generation, CodexAuthError::Io);
            return Err(CodexAuthError::Io);
        }
        let weak = Arc::downgrade(&self.inner);
        let deadline = self.inner.policy.graceful_shutdown_timeout;
        tokio::spawn(async move {
            tokio::time::sleep(deadline).await;
            if let Some(inner) = weak.upgrade() {
                inner.force(generation);
            }
        });
        Ok(())
    }

    pub async fn wait_until_idle(&self) -> Result<(), CodexAuthError> {
        let mut activity = self.inner.activity.subscribe();
        if !*activity.borrow() {
            return Ok(());
        }
        timeout(
            self.inner.policy.operation_timeout
                + self.inner.policy.graceful_shutdown_timeout
                + Duration::from_secs(1),
            async {
                while activity.changed().await.is_ok() {
                    if !*activity.borrow() {
                        return;
                    }
                }
            },
        )
        .await
        .map_err(|_| CodexAuthError::TimedOut)?;
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), CodexAuthError> {
        if self
            .inner
            .state
            .lock()
            .expect("authentication state lock was poisoned")
            .active
            .is_none()
        {
            return Ok(());
        }
        let _ = self.cancel().await;
        let stopped = timeout(
            self.inner.policy.graceful_shutdown_timeout + Duration::from_secs(1),
            self.wait_until_idle(),
        )
        .await;
        if !matches!(stopped, Ok(Ok(()))) {
            let generation = self
                .inner
                .state
                .lock()
                .expect("authentication state lock was poisoned")
                .active
                .as_ref()
                .map(|active| active.generation);
            if let Some(generation) = generation {
                self.inner.force(generation);
                timeout(
                    self.inner.policy.graceful_shutdown_timeout + Duration::from_secs(1),
                    self.wait_until_idle(),
                )
                .await
                .map_err(|_| CodexAuthError::TimedOut)??;
            }
        }
        Ok(())
    }

    pub(crate) fn fail_update_delivery(&self) {
        let generation = self
            .inner
            .state
            .lock()
            .expect("authentication state lock was poisoned")
            .active
            .as_ref()
            .map(|active| active.generation);
        if let Some(generation) = generation {
            self.inner.fail(generation, CodexAuthError::Protocol);
        }
    }
}

#[derive(Serialize)]
struct PromptResponse<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    id: &'a str,
    value: &'a str,
}

struct Inner {
    process: CodexAuthProcessSpec,
    policy: CodexAuthPolicy,
    launch_gate: AsyncMutex<()>,
    state: Mutex<State>,
    updates: broadcast::Sender<CodexAuthUpdate>,
    activity: watch::Sender<bool>,
}

impl Inner {
    fn apply(&self, generation: u64, update: CodexAuthUpdate) -> Result<(), CodexAuthError> {
        let update = match update {
            CodexAuthUpdate::Failed { .. } => CodexAuthUpdate::Failed {
                code: "sign_in_failed".into(),
                message: "Sign-in failed. Try again.".into(),
            },
            update => update,
        };
        let mut state = self
            .state
            .lock()
            .expect("authentication state lock was poisoned");
        let active = state.active.as_mut().ok_or(CodexAuthError::NotRunning)?;
        if active.generation != generation || active.terminal.is_some() {
            return Err(CodexAuthError::Protocol);
        }
        match &update {
            CodexAuthUpdate::Event { .. } => {}
            CodexAuthUpdate::Prompt { id, .. } => {
                if id.is_empty()
                    || !active.pending_prompts.is_empty()
                    || active.pending_prompts.contains(id)
                    || active.answered_prompts.contains(id)
                {
                    return Err(CodexAuthError::Protocol);
                }
                active.pending_prompts.insert(id.clone());
                state.status = CodexAuthStatus::WaitingForInput {
                    prompt_id: id.clone(),
                };
            }
            CodexAuthUpdate::PromptCancelled { id } => {
                if !active.pending_prompts.remove(id) && !active.answered_prompts.remove(id) {
                    return Err(CodexAuthError::Protocol);
                }
                state.status = CodexAuthStatus::SigningIn;
            }
            CodexAuthUpdate::Complete => {
                if !active.pending_prompts.is_empty() {
                    return Err(CodexAuthError::Protocol);
                }
                active.terminal = Some(TerminalOutcome::Complete);
                active.pending_prompts.clear();
                active.answered_prompts.clear();
                state.status = CodexAuthStatus::SigningIn;
            }
            CodexAuthUpdate::Cancelled => {
                if !active.pending_prompts.is_empty() {
                    return Err(CodexAuthError::Protocol);
                }
                active.terminal = Some(TerminalOutcome::Cancelled);
                active.pending_prompts.clear();
                active.answered_prompts.clear();
                state.status = CodexAuthStatus::Cancelled;
            }
            CodexAuthUpdate::Failed { message, .. } => {
                if !active.pending_prompts.is_empty() {
                    return Err(CodexAuthError::Protocol);
                }
                active.terminal = Some(TerminalOutcome::Failed);
                active.pending_prompts.clear();
                active.answered_prompts.clear();
                state.status = CodexAuthStatus::Failed {
                    code: CodexAuthFailureCode::SignInFailed,
                    message: message.clone(),
                };
            }
        }
        if !matches!(update, CodexAuthUpdate::Complete) {
            let _ = self.updates.send(update);
        }
        Ok(())
    }

    fn fail(&self, generation: u64, error: CodexAuthError) {
        let control = {
            let mut state = self
                .state
                .lock()
                .expect("authentication state lock was poisoned");
            let control = {
                let Some(active) = state.active.as_mut() else {
                    return;
                };
                if active.generation != generation {
                    return;
                }
                if active.terminal.is_some()
                    && active.terminal != Some(TerminalOutcome::Complete)
                    && !matches!(error, CodexAuthError::Protocol | CodexAuthError::Io)
                {
                    return;
                }
                if active.terminal == Some(TerminalOutcome::HostFailed) {
                    return;
                }
                active.terminal = Some(TerminalOutcome::HostFailed);
                active.pending_prompts.clear();
                active.answered_prompts.clear();
                active.control.clone()
            };
            let (status_code, protocol_code, message) = match error {
                CodexAuthError::TimedOut => (
                    CodexAuthFailureCode::SignInTimedOut,
                    "sign_in_timed_out",
                    "Sign-in timed out. Try again.",
                ),
                _ => (
                    CodexAuthFailureCode::SignInFailed,
                    "sign_in_failed",
                    "Sign-in failed. Try again.",
                ),
            };
            state.status = CodexAuthStatus::Failed {
                code: status_code,
                message: message.into(),
            };
            let _ = self.updates.send(CodexAuthUpdate::Failed {
                code: protocol_code.into(),
                message: message.into(),
            });
            control
        };
        let _ = control.try_send(SupervisorCommand::Force);
    }

    fn force(&self, generation: u64) {
        let control = self
            .state
            .lock()
            .expect("authentication state lock was poisoned")
            .active
            .as_ref()
            .filter(|active| active.generation == generation)
            .map(|active| active.control.clone());
        if let Some(control) = control {
            let _ = control.try_send(SupervisorCommand::Force);
        }
    }

    fn process_exited(&self, generation: u64, code: Option<i32>, signal: Option<i32>) {
        let mut state = self
            .state
            .lock()
            .expect("authentication state lock was poisoned");
        let Some(active) = state.active.as_ref() else {
            return;
        };
        if active.generation != generation {
            return;
        }
        let valid_completion = active.terminal == Some(TerminalOutcome::Complete)
            && code == Some(0)
            && signal.is_none();
        if valid_completion {
            state.status = CodexAuthStatus::SignedIn;
            let _ = self.updates.send(CodexAuthUpdate::Complete);
        } else if active.terminal == Some(TerminalOutcome::Complete) {
            state.status = CodexAuthStatus::Failed {
                code: CodexAuthFailureCode::SignInFailed,
                message: "Sign-in failed. Try again.".into(),
            };
            let _ = self.updates.send(CodexAuthUpdate::Failed {
                code: "sign_in_failed".into(),
                message: "Sign-in failed. Try again.".into(),
            });
        } else if active.terminal.is_none() {
            if active.cancel_requested {
                state.status = CodexAuthStatus::Cancelled;
                let _ = self.updates.send(CodexAuthUpdate::Cancelled);
            } else {
                state.status = CodexAuthStatus::Failed {
                    code: CodexAuthFailureCode::SignInFailed,
                    message: "Sign-in failed. Try again.".into(),
                };
                let _ = self.updates.send(CodexAuthUpdate::Failed {
                    code: "sign_in_failed".into(),
                    message: "Sign-in failed. Try again.".into(),
                });
            }
        }
        state.active = None;
        self.activity.send_replace(false);
        tracing::debug!(?code, ?signal, "authentication helper stopped");
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(state) = self.state.lock()
            && let Some(active) = &state.active
        {
            active.process_group.force_kill();
        }
    }
}

#[derive(Default)]
struct State {
    status: CodexAuthStatus,
    active: Option<Active>,
    next_generation: u64,
}

struct Active {
    generation: u64,
    process_group: OwnedProcessGroup,
    writer: mpsc::Sender<Vec<u8>>,
    control: mpsc::Sender<SupervisorCommand>,
    pending_prompts: BTreeSet<String>,
    answered_prompts: BTreeSet<String>,
    cancel_requested: bool,
    terminal: Option<TerminalOutcome>,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum TerminalOutcome {
    Complete,
    Cancelled,
    Failed,
    HostFailed,
}

#[derive(Copy, Clone)]
enum SupervisorCommand {
    Force,
}

#[allow(clippy::too_many_arguments)]
fn spawn_tasks(
    inner: Weak<Inner>,
    generation: u64,
    process: Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    process_group: OwnedProcessGroup,
    writer_receiver: mpsc::Receiver<Vec<u8>>,
    control_receiver: mpsc::Receiver<SupervisorCommand>,
) {
    let writer_inner = inner.clone();
    tokio::spawn(write_stdin(
        stdin,
        writer_receiver,
        writer_inner,
        generation,
    ));
    let reader_inner = inner.clone();
    let stdout_reader = tokio::spawn(read_stdout(stdout, reader_inner, generation));
    let stderr_reader = tokio::spawn(drain_stderr(stderr));
    tokio::spawn(supervise(
        process,
        control_receiver,
        inner,
        generation,
        stdout_reader,
        stderr_reader,
        process_group,
    ));
}

async fn write_stdin(
    mut stdin: tokio::process::ChildStdin,
    mut receiver: mpsc::Receiver<Vec<u8>>,
    inner: Weak<Inner>,
    generation: u64,
) {
    while let Some(frame) = receiver.recv().await {
        if stdin.write_all(&frame).await.is_err() || stdin.flush().await.is_err() {
            if let Some(inner) = inner.upgrade() {
                inner.fail(generation, CodexAuthError::Io);
            }
            return;
        }
    }
    let _ = stdin.shutdown().await;
}

async fn read_stdout(mut stdout: tokio::process::ChildStdout, inner: Weak<Inner>, generation: u64) {
    let Some(owner) = inner.upgrade() else {
        return;
    };
    let maximum_record_bytes = owner.policy.maximum_record_bytes;
    drop(owner);
    let mut buffered = Vec::with_capacity(8 * 1024);
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = match stdout.read(&mut chunk).await {
            Ok(read) => read,
            Err(_) => {
                if let Some(inner) = inner.upgrade() {
                    inner.fail(generation, CodexAuthError::Io);
                }
                return;
            }
        };
        if read == 0 {
            if !buffered.is_empty()
                && process_record(&mut buffered, maximum_record_bytes, &inner, generation).is_err()
            {
                return;
            }
            return;
        }
        buffered.extend_from_slice(&chunk[..read]);
        while let Some(newline) = buffered.iter().position(|byte| *byte == b'\n') {
            let mut record = buffered.drain(..=newline).collect::<Vec<_>>();
            record.pop();
            if process_record(&mut record, maximum_record_bytes, &inner, generation).is_err() {
                return;
            }
        }
        if buffered.len() > maximum_record_bytes {
            if let Some(inner) = inner.upgrade() {
                inner.fail(generation, CodexAuthError::Protocol);
            }
            return;
        }
    }
}

fn process_record(
    record: &mut Vec<u8>,
    maximum_record_bytes: usize,
    inner: &Weak<Inner>,
    generation: u64,
) -> Result<(), ()> {
    if record.last() == Some(&b'\r') {
        record.pop();
    }
    if record.is_empty() || record.len() > maximum_record_bytes {
        if let Some(inner) = inner.upgrade() {
            inner.fail(generation, CodexAuthError::Protocol);
        }
        return Err(());
    }
    let update = std::str::from_utf8(record).ok().and_then(parse_update);
    let Some(update) = update else {
        if let Some(inner) = inner.upgrade() {
            inner.fail(generation, CodexAuthError::Protocol);
        }
        return Err(());
    };
    let terminal = matches!(
        update,
        CodexAuthUpdate::Complete | CodexAuthUpdate::Cancelled | CodexAuthUpdate::Failed { .. }
    );
    let Some(inner) = inner.upgrade() else {
        return Err(());
    };
    if inner.apply(generation, update).is_err() {
        inner.fail(generation, CodexAuthError::Protocol);
        return Err(());
    }
    if terminal {
        let weak = Arc::downgrade(&inner);
        let deadline = inner.policy.graceful_shutdown_timeout;
        tokio::spawn(async move {
            tokio::time::sleep(deadline).await;
            if let Some(inner) = weak.upgrade() {
                inner.force(generation);
            }
        });
    }
    Ok(())
}

fn parse_update(record: &str) -> Option<CodexAuthUpdate> {
    let value = serde_json::from_str::<serde_json::Value>(record).ok()?;
    if !valid_update_shape(&value) {
        return None;
    }
    serde_json::from_value(value).ok()
}

fn valid_update_shape(value: &serde_json::Value) -> bool {
    let Some(record) = value.as_object() else {
        return false;
    };
    match record.get("type").and_then(serde_json::Value::as_str) {
        Some("auth_event") => {
            exact_keys(record, &["type", "event"], &[])
                && record.get("event").is_some_and(valid_event_shape)
        }
        Some("auth_prompt") => {
            exact_keys(record, &["type", "id", "prompt"], &[])
                && non_empty_string(record.get("id"))
                && record.get("prompt").is_some_and(valid_prompt_shape)
        }
        Some("auth_prompt_cancelled") => {
            exact_keys(record, &["type", "id"], &[]) && non_empty_string(record.get("id"))
        }
        Some("auth_complete" | "auth_cancelled") => exact_keys(record, &["type"], &[]),
        Some("auth_failed") => {
            exact_keys(record, &["type", "code", "message"], &[])
                && non_empty_string(record.get("code"))
                && non_empty_string(record.get("message"))
        }
        _ => false,
    }
}

fn valid_event_shape(value: &serde_json::Value) -> bool {
    let Some(event) = value.as_object() else {
        return false;
    };
    match event.get("type").and_then(serde_json::Value::as_str) {
        Some("info") => {
            exact_keys(event, &["type", "message"], &["links"])
                && non_empty_string(event.get("message"))
                && event.get("links").is_none_or(|links| {
                    links.as_array().is_some_and(|links| {
                        links.iter().all(|link| {
                            link.as_object().is_some_and(|link| {
                                exact_keys(link, &["url"], &["label"])
                                    && non_empty_string(link.get("url"))
                                    && optional_non_empty_string(link.get("label"))
                            })
                        })
                    })
                })
        }
        Some("auth_url") => {
            exact_keys(event, &["type", "url"], &["instructions"])
                && non_empty_string(event.get("url"))
                && optional_non_empty_string(event.get("instructions"))
        }
        Some("device_code") => {
            exact_keys(
                event,
                &["type", "userCode", "verificationUri"],
                &["intervalSeconds", "expiresInSeconds"],
            ) && non_empty_string(event.get("userCode"))
                && non_empty_string(event.get("verificationUri"))
                && optional_number(event.get("intervalSeconds"))
                && optional_number(event.get("expiresInSeconds"))
        }
        Some("progress") => {
            exact_keys(event, &["type", "message"], &[]) && non_empty_string(event.get("message"))
        }
        _ => false,
    }
}

fn valid_prompt_shape(value: &serde_json::Value) -> bool {
    let Some(prompt) = value.as_object() else {
        return false;
    };
    match prompt.get("type").and_then(serde_json::Value::as_str) {
        Some("select") => {
            exact_keys(prompt, &["type", "message", "options"], &[])
                && non_empty_string(prompt.get("message"))
                && prompt.get("options").is_some_and(|options| {
                    options
                        .as_array()
                        .filter(|options| !options.is_empty())
                        .is_some_and(|options| {
                            options.iter().all(|option| {
                                option.as_object().is_some_and(|option| {
                                    exact_keys(option, &["id", "label"], &["description"])
                                        && non_empty_string(option.get("id"))
                                        && non_empty_string(option.get("label"))
                                        && optional_non_empty_string(option.get("description"))
                                })
                            })
                        })
                })
        }
        Some("text" | "secret" | "manual_code") => {
            exact_keys(prompt, &["type", "message"], &["placeholder"])
                && non_empty_string(prompt.get("message"))
                && optional_non_empty_string(prompt.get("placeholder"))
        }
        _ => false,
    }
}

fn exact_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    required: &[&str],
    optional: &[&str],
) -> bool {
    required.iter().all(|key| object.contains_key(*key))
        && object
            .keys()
            .all(|key| required.contains(&key.as_str()) || optional.contains(&key.as_str()))
}

fn non_empty_string(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.is_empty())
}

fn optional_non_empty_string(value: Option<&serde_json::Value>) -> bool {
    value.is_none_or(|value| value.as_str().is_some_and(|value| !value.is_empty()))
}

fn optional_number(value: Option<&serde_json::Value>) -> bool {
    value.is_none_or(serde_json::Value::is_number)
}

async fn drain_stderr(mut stderr: impl AsyncRead + Unpin) {
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

async fn supervise(
    mut process: Child,
    mut controls: mpsc::Receiver<SupervisorCommand>,
    inner: Weak<Inner>,
    generation: u64,
    stdout_reader: tokio::task::JoinHandle<()>,
    stderr_reader: tokio::task::JoinHandle<()>,
    process_group: OwnedProcessGroup,
) {
    let status = tokio::select! {
        status = process.wait() => status.ok(),
        _ = controls.recv() => {
            process_group.force_kill();
            process.wait().await.ok()
        }
    };
    process_group.force_kill();
    let _ = stdout_reader.await;
    let _ = stderr_reader.await;
    if let Some(inner) = inner.upgrade() {
        inner.process_exited(
            generation,
            status.as_ref().and_then(std::process::ExitStatus::code),
            status.as_ref().and_then(std::process::ExitStatus::signal),
        );
    }
}

async fn create_credential_lock_directory(arguments: &[OsString]) -> Result<(), CodexAuthError> {
    let Some(index) = arguments
        .iter()
        .position(|argument| argument == OsStr::new("--credential-lock-dir"))
    else {
        return Err(CodexAuthError::Spawn);
    };
    let Some(path) = arguments.get(index + 1) else {
        return Err(CodexAuthError::Spawn);
    };
    tokio::fs::create_dir_all(PathBuf::from(path))
        .await
        .map_err(|_| CodexAuthError::Spawn)
}

fn validate(
    process: &CodexAuthProcessSpec,
    policy: &CodexAuthPolicy,
) -> Result<(), CodexAuthError> {
    if !process.executable.is_absolute() || !process.working_directory.is_absolute() {
        return Err(CodexAuthError::NonAbsoluteProcessPath);
    }
    if process
        .arguments
        .first()
        .is_none_or(|launcher| !Path::new(launcher).is_absolute())
    {
        return Err(CodexAuthError::NonAbsoluteProcessPath);
    }
    if process
        .environment
        .get(OsStr::new("HOME"))
        .is_none_or(|home| home.is_empty())
    {
        return Err(CodexAuthError::MissingHome);
    }
    if policy.operation_timeout.is_zero()
        || policy.graceful_shutdown_timeout.is_zero()
        || policy.maximum_record_bytes == 0
        || policy.maximum_response_bytes == 0
        || policy.write_queue_capacity == 0
        || policy.update_queue_capacity == 0
    {
        return Err(CodexAuthError::InvalidPolicy);
    }
    Ok(())
}

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    ffi::OsString,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::Value;
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Child,
    sync::{broadcast, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, sleep_until, timeout},
};
use tokio_util::sync::CancellationToken;

use crate::owned_process::{OwnedProcessGroup, spawn_owned_piped_process};

const MAX_ABANDONED_REQUESTS: usize = 256;

/// The complete, explicit invocation of one Più-owned Pi child.
#[derive(Clone, Debug)]
pub struct PiRpcProcessSpec {
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
}

/// Host-only operational limits. These are application policy, not user settings.
#[derive(Clone, Debug)]
pub struct PiRpcPolicy {
    pub readiness_timeout: Duration,
    pub request_timeout: Duration,
    pub graceful_shutdown_timeout: Duration,
    pub maximum_record_bytes: usize,
    pub retained_stderr_bytes: usize,
    pub write_queue_capacity: usize,
    pub event_queue_capacity: usize,
}

impl Default for PiRpcPolicy {
    fn default() -> Self {
        Self {
            readiness_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(30),
            graceful_shutdown_timeout: Duration::from_secs(5),
            maximum_record_bytes: 16 * 1024 * 1024,
            retained_stderr_bytes: 256 * 1024,
            write_queue_capacity: 64,
            event_queue_capacity: 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PiRpcResponse {
    pub command: String,
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PiRpcEvent {
    pub kind: String,
    pub payload: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PiRpcDiagnostics {
    pub stderr: String,
    pub stderr_was_truncated: bool,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PiRpcError {
    #[error("Pi child executable and working directory must use absolute paths")]
    NonAbsoluteProcessPath,
    #[error("Pi transport limits must be greater than zero")]
    InvalidPolicy,
    #[error("could not start the Pi child: {0}")]
    Spawn(String),
    #[error("Pi did not become ready before the startup deadline")]
    ReadinessTimedOut,
    #[error("Pi exited before readiness: {0}")]
    ReadinessFailed(String),
    #[error("request {command} timed out")]
    RequestTimedOut { command: String },
    #[error("request {command} was cancelled")]
    RequestCancelled { command: String },
    #[error("Pi rejected {command}: {message}")]
    Remote { command: String, message: String },
    #[error("invalid Pi command: {0}")]
    InvalidCommand(String),
    #[error("Pi protocol violation: {0}")]
    Protocol(String),
    #[error("Pi child I/O failed: {0}")]
    Io(String),
    #[error("Pi child exited (code {code:?}, signal {signal:?})")]
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    #[error("Pi child is stopped")]
    Stopped,
    #[error("event consumer fell behind by {missed} records")]
    EventBackpressure { missed: u64 },
}

/// A bounded parsed-event subscription. Unknown event kinds remain visible to callers.
pub struct PiRpcEvents {
    receiver: broadcast::Receiver<PiRpcEvent>,
    terminal: watch::Receiver<Option<PiRpcError>>,
}

impl PiRpcEvents {
    pub async fn recv(&mut self) -> Result<PiRpcEvent, PiRpcError> {
        match self.receiver.try_recv() {
            Ok(event) => return Ok(event),
            Err(broadcast::error::TryRecvError::Lagged(missed)) => {
                return Err(PiRpcError::EventBackpressure { missed });
            }
            Err(broadcast::error::TryRecvError::Closed) => return Err(PiRpcError::Stopped),
            Err(broadcast::error::TryRecvError::Empty) => {}
        }
        if let Some(error) = self.terminal.borrow().clone() {
            return Err(error);
        }
        tokio::select! {
            biased;
            result = self.receiver.recv() => match result {
                Ok(event) => Ok(event),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    Err(PiRpcError::EventBackpressure { missed })
                }
                Err(broadcast::error::RecvError::Closed) => Err(PiRpcError::Stopped),
            },
            changed = self.terminal.changed() => {
                if changed.is_err() {
                    Err(PiRpcError::Stopped)
                } else {
                    Err(self.terminal.borrow().clone().unwrap_or(PiRpcError::Stopped))
                }
            }
        }
    }
}

/// Owns one Pi child and hides JSONL, correlation, pipe, and process-group invariants.
pub struct PiRpcChild {
    shared: Arc<Shared>,
    supervisor_sender: mpsc::Sender<SupervisorCommand>,
    supervisor: Mutex<Option<JoinHandle<Result<(), PiRpcError>>>>,
    io_tasks: Mutex<Option<Vec<JoinHandle<()>>>>,
    process_group: OwnedProcessGroup,
    request_timeout: Duration,
    graceful_shutdown_timeout: Duration,
}

impl PiRpcChild {
    pub async fn launch(spec: PiRpcProcessSpec, policy: PiRpcPolicy) -> Result<Self, PiRpcError> {
        validate(&spec, &policy)?;

        let process = spawn_owned_piped_process(
            &spec.executable,
            &spec.arguments,
            &spec.working_directory,
            &spec.environment,
        )
        .map_err(|error| PiRpcError::Spawn(error.to_string()))?;
        let process_group = process.process_group;

        let (writer_sender, writer_receiver) = mpsc::channel(policy.write_queue_capacity);
        let (event_sender, _) = broadcast::channel(policy.event_queue_capacity);
        let (terminal_sender, _) = watch::channel(None);
        let (supervisor_sender, supervisor_receiver) = mpsc::channel(4);
        let writer_stop = CancellationToken::new();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            writer: writer_sender,
            events: event_sender,
            terminal: terminal_sender,
            supervisor: supervisor_sender.clone(),
            writer_stop: writer_stop.clone(),
            next_request: Mutex::new(0),
            stderr: Mutex::new(BoundedStderr::new(policy.retained_stderr_bytes)),
        });

        let writer = tokio::spawn(write_stdin(
            process.stdin,
            writer_receiver,
            writer_stop,
            Arc::clone(&shared),
        ));
        let stdout_reader = tokio::spawn(read_stdout(
            process.stdout,
            policy.maximum_record_bytes,
            Arc::clone(&shared),
        ));
        let stderr_reader = tokio::spawn(read_stderr(process.stderr, Arc::clone(&shared)));
        let supervisor_shared = Arc::clone(&shared);
        let supervisor = tokio::spawn(supervise(
            process.child,
            supervisor_receiver,
            supervisor_shared,
            process_group,
            policy.graceful_shutdown_timeout,
        ));
        let child = Self {
            shared,
            supervisor_sender,
            supervisor: Mutex::new(Some(supervisor)),
            io_tasks: Mutex::new(Some(vec![writer, stdout_reader, stderr_reader])),
            process_group,
            request_timeout: policy.request_timeout,
            graceful_shutdown_timeout: policy.graceful_shutdown_timeout,
        };

        let readiness = child
            .request_with_timeout(
                serde_json::json!({ "type": "get_state" }),
                CancellationToken::new(),
                policy.readiness_timeout,
            )
            .await;
        match readiness {
            Ok(response) if response.command == "get_state" => Ok(child),
            Ok(_) => {
                child.shutdown_after_launch_failure().await;
                Err(PiRpcError::ReadinessFailed(
                    "readiness response did not acknowledge get_state".into(),
                ))
            }
            Err(PiRpcError::RequestTimedOut { .. }) => {
                child.shutdown_after_launch_failure().await;
                Err(PiRpcError::ReadinessTimedOut)
            }
            Err(error) => {
                child.shutdown_after_launch_failure().await;
                Err(PiRpcError::ReadinessFailed(error.to_string()))
            }
        }
    }

    pub fn subscribe(&self) -> PiRpcEvents {
        PiRpcEvents {
            receiver: self.shared.events.subscribe(),
            terminal: self.shared.terminal.subscribe(),
        }
    }

    pub async fn request(
        &self,
        command: Value,
        cancellation: CancellationToken,
    ) -> Result<PiRpcResponse, PiRpcError> {
        self.request_with_timeout(command, cancellation, self.request_timeout)
            .await
    }

    pub fn diagnostics(&self) -> PiRpcDiagnostics {
        self.shared
            .stderr
            .lock()
            .expect("stderr buffer lock was poisoned")
            .snapshot()
    }

    pub async fn shutdown(&self) -> Result<(), PiRpcError> {
        self.shared.finish(PiRpcError::Stopped);
        self.shared.writer_stop.cancel();
        let _ = self
            .supervisor_sender
            .send(SupervisorCommand::Graceful)
            .await;
        let supervisor = self
            .supervisor
            .lock()
            .expect("supervisor lock was poisoned")
            .take();
        let Some(supervisor) = supervisor else {
            return Ok(());
        };
        let supervision = match timeout(
            self.graceful_shutdown_timeout + Duration::from_secs(1),
            supervisor,
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(PiRpcError::Io(format!(
                "process supervisor task failed: {error}"
            ))),
            Err(_) => {
                self.process_group.force_kill();
                Err(PiRpcError::Io(
                    "process supervisor did not finish after forced shutdown".into(),
                ))
            }
        };
        let pipes = self.finish_io_tasks().await;
        supervision.and(pipes)
    }

    async fn request_with_timeout(
        &self,
        mut command: Value,
        cancellation: CancellationToken,
        request_timeout: Duration,
    ) -> Result<PiRpcResponse, PiRpcError> {
        let object = command.as_object_mut().ok_or_else(|| {
            PiRpcError::InvalidCommand("the command must be a JSON object".into())
        })?;
        let command_name = object
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty() && *kind != "response")
            .ok_or_else(|| {
                PiRpcError::InvalidCommand("the command needs a non-response type".into())
            })?
            .to_owned();
        if object.contains_key("id") {
            return Err(PiRpcError::InvalidCommand(
                "request IDs are owned by the transport".into(),
            ));
        }
        let id = self.shared.next_id();
        object.insert("id".into(), Value::String(id.clone()));
        let mut frame = serde_json::to_vec(&command)
            .map_err(|error| PiRpcError::InvalidCommand(error.to_string()))?;
        frame.push(b'\n');
        let (response_sender, response_receiver) = oneshot::channel();
        self.shared
            .insert_pending(id.clone(), command_name.clone(), response_sender)?;
        let deadline = Instant::now() + request_timeout;

        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                self.shared.discard(&id);
                return Err(PiRpcError::RequestCancelled { command: command_name });
            }
            _ = sleep_until(deadline) => {
                self.shared.discard(&id);
                return Err(PiRpcError::RequestTimedOut { command: command_name });
            }
            sent = self.shared.writer.send(frame) => {
                if sent.is_err() {
                    self.shared.discard(&id);
                    return Err(self.shared.terminal_error().unwrap_or(PiRpcError::Stopped));
                }
            }
        }

        tokio::select! {
            biased;
            result = response_receiver => {
                result.unwrap_or(Err(PiRpcError::Stopped))
            }
            _ = cancellation.cancelled() => {
                self.shared.abandon(&id);
                Err(PiRpcError::RequestCancelled { command: command_name })
            }
            _ = sleep_until(deadline) => {
                self.shared.abandon(&id);
                Err(PiRpcError::RequestTimedOut { command: command_name })
            }
        }
    }

    async fn shutdown_after_launch_failure(&self) {
        self.shared.finish(PiRpcError::Stopped);
        self.shared.writer_stop.cancel();
        let _ = self.supervisor_sender.send(SupervisorCommand::Force).await;
        let supervisor = self
            .supervisor
            .lock()
            .expect("supervisor lock was poisoned")
            .take();
        if let Some(supervisor) = supervisor {
            let _ = timeout(Duration::from_secs(2), supervisor).await;
        }
        let _ = self.finish_io_tasks().await;
    }

    async fn finish_io_tasks(&self) -> Result<(), PiRpcError> {
        let tasks = self
            .io_tasks
            .lock()
            .expect("I/O task lock was poisoned")
            .take();
        let Some(tasks) = tasks else {
            return Ok(());
        };
        for mut task in tasks {
            match timeout(Duration::from_secs(1), &mut task).await {
                Ok(result) => result
                    .map_err(|error| PiRpcError::Io(format!("child pipe task failed: {error}")))?,
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                }
            }
        }
        Ok(())
    }
}

impl Drop for PiRpcChild {
    fn drop(&mut self) {
        let supervisor_is_running = self
            .supervisor
            .lock()
            .expect("supervisor lock was poisoned")
            .as_ref()
            .is_some_and(|supervisor| !supervisor.is_finished());
        if supervisor_is_running {
            self.shared.finish(PiRpcError::Stopped);
            self.shared.writer_stop.cancel();
            let _ = self.supervisor_sender.try_send(SupervisorCommand::Force);
            self.process_group.force_kill();
        }
    }
}

#[derive(Default)]
struct State {
    terminal: Option<PiRpcError>,
    pending: HashMap<String, PendingRequest>,
    abandoned: VecDeque<String>,
}

struct PendingRequest {
    command: String,
    sender: oneshot::Sender<Result<PiRpcResponse, PiRpcError>>,
}

struct Shared {
    state: Mutex<State>,
    writer: mpsc::Sender<Vec<u8>>,
    events: broadcast::Sender<PiRpcEvent>,
    terminal: watch::Sender<Option<PiRpcError>>,
    supervisor: mpsc::Sender<SupervisorCommand>,
    writer_stop: CancellationToken,
    next_request: Mutex<u64>,
    stderr: Mutex<BoundedStderr>,
}

impl Shared {
    fn next_id(&self) -> String {
        let mut next = self
            .next_request
            .lock()
            .expect("request counter lock was poisoned");
        *next += 1;
        format!("piu-{}", *next)
    }

    fn insert_pending(
        &self,
        id: String,
        command: String,
        sender: oneshot::Sender<Result<PiRpcResponse, PiRpcError>>,
    ) -> Result<(), PiRpcError> {
        let mut state = self
            .state
            .lock()
            .expect("transport state lock was poisoned");
        if let Some(error) = &state.terminal {
            return Err(error.clone());
        }
        state.pending.insert(id, PendingRequest { command, sender });
        Ok(())
    }

    fn abandon(&self, id: &str) {
        let mut state = self
            .state
            .lock()
            .expect("transport state lock was poisoned");
        if state.pending.remove(id).is_some() {
            state.abandoned.push_back(id.to_owned());
            if state.abandoned.len() > MAX_ABANDONED_REQUESTS {
                state.abandoned.pop_front();
            }
        }
    }

    fn discard(&self, id: &str) {
        self.state
            .lock()
            .expect("transport state lock was poisoned")
            .pending
            .remove(id);
    }

    fn resolve_response(&self, value: Value) -> Result<(), PiRpcError> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| PiRpcError::Protocol("response omitted its string id".into()))?
            .to_owned();
        let response_command = value
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| PiRpcError::Protocol("response omitted its command".into()))?
            .to_owned();
        let success = value
            .get("success")
            .and_then(Value::as_bool)
            .ok_or_else(|| PiRpcError::Protocol("response omitted its success flag".into()))?;

        let pending = {
            let mut state = self
                .state
                .lock()
                .expect("transport state lock was poisoned");
            if let Some(index) = state
                .abandoned
                .iter()
                .position(|candidate| candidate == &id)
            {
                state.abandoned.remove(index);
                return Ok(());
            }
            state.pending.remove(&id)
        };
        let Some(pending) = pending else {
            return Err(PiRpcError::Protocol(format!(
                "unknown or duplicate response id {id}"
            )));
        };
        if pending.command != response_command {
            let error = PiRpcError::Protocol(format!(
                "response {id} acknowledged {response_command}, expected {}",
                pending.command
            ));
            let _ = pending.sender.send(Err(error.clone()));
            return Err(error);
        }
        let response = if success {
            Ok(PiRpcResponse {
                command: response_command,
                data: value.get("data").cloned(),
            })
        } else {
            let Some(message) = value.get("error").and_then(Value::as_str) else {
                let error =
                    PiRpcError::Protocol(format!("failed response {id} omitted its error message"));
                let _ = pending.sender.send(Err(error.clone()));
                return Err(error);
            };
            Err(PiRpcError::Remote {
                command: response_command,
                message: message.to_owned(),
            })
        };
        let _ = pending.sender.send(response);
        Ok(())
    }

    fn finish(&self, error: PiRpcError) {
        let pending = {
            let mut state = self
                .state
                .lock()
                .expect("transport state lock was poisoned");
            if state.terminal.is_some() {
                return;
            }
            state.terminal = Some(error.clone());
            std::mem::take(&mut state.pending)
        };
        self.terminal.send_replace(Some(error.clone()));
        for (_, pending) in pending {
            let _ = pending.sender.send(Err(error.clone()));
        }
    }

    fn fail(&self, error: PiRpcError) {
        self.finish(error);
        self.writer_stop.cancel();
        let _ = self.supervisor.try_send(SupervisorCommand::Force);
    }

    fn terminal_error(&self) -> Option<PiRpcError> {
        self.state
            .lock()
            .expect("transport state lock was poisoned")
            .terminal
            .clone()
    }
}

#[derive(Copy, Clone)]
enum SupervisorCommand {
    Graceful,
    Force,
}

async fn write_stdin(
    mut stdin: tokio::process::ChildStdin,
    mut receiver: mpsc::Receiver<Vec<u8>>,
    stop: CancellationToken,
    shared: Arc<Shared>,
) {
    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            frame = receiver.recv() => {
                let Some(frame) = frame else { break };
                if let Err(error) = stdin.write_all(&frame).await {
                    shared.fail(PiRpcError::Io(format!("could not write child stdin: {error}")));
                    return;
                }
                if let Err(error) = stdin.flush().await {
                    shared.fail(PiRpcError::Io(format!("could not flush child stdin: {error}")));
                    return;
                }
            }
        }
    }
    let _ = stdin.shutdown().await;
}

async fn read_stdout(
    mut stdout: tokio::process::ChildStdout,
    maximum_record_bytes: usize,
    shared: Arc<Shared>,
) {
    let mut buffered = Vec::with_capacity(8 * 1024);
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = match stdout.read(&mut chunk).await {
            Ok(read) => read,
            Err(error) => {
                shared.fail(PiRpcError::Io(format!(
                    "could not read child stdout: {error}"
                )));
                return;
            }
        };
        if read == 0 {
            if !buffered.is_empty()
                && process_record(&mut buffered, maximum_record_bytes, &shared).is_err()
            {
                return;
            }
            shared.fail(PiRpcError::Protocol("child stdout reached EOF".into()));
            return;
        }
        buffered.extend_from_slice(&chunk[..read]);
        while let Some(newline) = buffered.iter().position(|byte| *byte == b'\n') {
            let mut record = buffered.drain(..=newline).collect::<Vec<_>>();
            record.pop();
            if process_record(&mut record, maximum_record_bytes, &shared).is_err() {
                return;
            }
        }
        if buffered.len() > maximum_record_bytes {
            shared.fail(PiRpcError::Protocol(format!(
                "stdout record exceeded {maximum_record_bytes} bytes"
            )));
            return;
        }
    }
}

fn process_record(
    record: &mut Vec<u8>,
    maximum_record_bytes: usize,
    shared: &Shared,
) -> Result<(), ()> {
    if record.last() == Some(&b'\r') {
        record.pop();
    }
    if record.len() > maximum_record_bytes {
        shared.fail(PiRpcError::Protocol(format!(
            "stdout record exceeded {maximum_record_bytes} bytes"
        )));
        return Err(());
    }
    let text = match std::str::from_utf8(record) {
        Ok(text) => text,
        Err(_) => {
            shared.fail(PiRpcError::Protocol(
                "stdout record was not valid UTF-8".into(),
            ));
            return Err(());
        }
    };
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            shared.fail(PiRpcError::Protocol(
                "stdout record was not a JSON object".into(),
            ));
            return Err(());
        }
    };
    let Some(object) = value.as_object() else {
        shared.fail(PiRpcError::Protocol(
            "stdout record was not a JSON object".into(),
        ));
        return Err(());
    };
    let Some(kind) = object.get("type").and_then(Value::as_str) else {
        shared.fail(PiRpcError::Protocol(
            "stdout object omitted its string type".into(),
        ));
        return Err(());
    };
    if kind == "response" {
        if let Err(error) = shared.resolve_response(value) {
            shared.fail(error);
            return Err(());
        }
    } else {
        let _ = shared.events.send(PiRpcEvent {
            kind: kind.to_owned(),
            payload: value,
        });
    }
    Ok(())
}

async fn read_stderr(mut stderr: impl AsyncRead + Unpin, shared: Arc<Shared>) {
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) => return,
            Ok(read) => shared
                .stderr
                .lock()
                .expect("stderr buffer lock was poisoned")
                .append(&chunk[..read]),
            Err(error) => {
                shared.fail(PiRpcError::Io(format!(
                    "could not read child stderr: {error}"
                )));
                return;
            }
        }
    }
}

async fn supervise(
    mut process: Child,
    mut commands: mpsc::Receiver<SupervisorCommand>,
    shared: Arc<Shared>,
    process_group: OwnedProcessGroup,
    graceful_shutdown_timeout: Duration,
) -> Result<(), PiRpcError> {
    tokio::select! {
        status = process.wait() => {
            let status = status.map_err(|error| PiRpcError::Io(format!("could not wait for Pi child: {error}")))?;
            process_group.force_kill();
            shared.finish(PiRpcError::Exited {
                code: status.code(),
                signal: status.signal(),
            });
            Ok(())
        }
        command = commands.recv() => match command {
            Some(SupervisorCommand::Graceful) => {
                shared.writer_stop.cancel();
                match timeout(graceful_shutdown_timeout, process.wait()).await {
                    Ok(Ok(_)) => {
                        process_group.force_kill();
                        Ok(())
                    }
                    Ok(Err(error)) => Err(PiRpcError::Io(format!("could not wait for Pi child: {error}"))),
                    Err(_) => {
                        process_group.force_kill();
                        process.wait().await.map_err(|error| PiRpcError::Io(format!("could not reap Pi child: {error}")))?;
                        // The leader can fork between the first signal and its death.
                        // Once reaped, a second group signal catches those descendants.
                        process_group.force_kill();
                        Ok(())
                    }
                }
            }
            Some(SupervisorCommand::Force) | None => {
                shared.writer_stop.cancel();
                process_group.force_kill();
                process.wait().await.map_err(|error| PiRpcError::Io(format!("could not reap Pi child: {error}")))?;
                process_group.force_kill();
                Ok(())
            }
        }
    }
}

fn validate(spec: &PiRpcProcessSpec, policy: &PiRpcPolicy) -> Result<(), PiRpcError> {
    if !spec.executable.is_absolute() || !spec.working_directory.is_absolute() {
        return Err(PiRpcError::NonAbsoluteProcessPath);
    }
    if policy.readiness_timeout.is_zero()
        || policy.request_timeout.is_zero()
        || policy.graceful_shutdown_timeout.is_zero()
        || policy.maximum_record_bytes == 0
        || policy.retained_stderr_bytes == 0
        || policy.write_queue_capacity == 0
        || policy.event_queue_capacity == 0
    {
        return Err(PiRpcError::InvalidPolicy);
    }
    Ok(())
}

struct BoundedStderr {
    bytes: VecDeque<u8>,
    capacity: usize,
    truncated: bool,
}

impl BoundedStderr {
    fn new(capacity: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(capacity.min(8 * 1024)),
            capacity,
            truncated: false,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        if chunk.len() >= self.capacity {
            self.bytes.clear();
            self.bytes
                .extend(chunk[chunk.len() - self.capacity..].iter().copied());
            self.truncated = true;
            return;
        }
        let overflow = self
            .bytes
            .len()
            .saturating_add(chunk.len())
            .saturating_sub(self.capacity);
        if overflow > 0 {
            self.bytes.drain(..overflow);
            self.truncated = true;
        }
        self.bytes.extend(chunk.iter().copied());
    }

    fn snapshot(&self) -> PiRpcDiagnostics {
        let bytes = self.bytes.iter().copied().collect::<Vec<_>>();
        PiRpcDiagnostics {
            stderr: String::from_utf8_lossy(&bytes).into_owned(),
            stderr_was_truncated: self.truncated,
        }
    }
}

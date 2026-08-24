use std::{
    collections::HashMap,
    fs,
    io::Read,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use rustix::process::{Pid, Signal, kill_process_group};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::{
    git_process::{GitProcess, GitProcessError},
    project_inbox::{
        ChatCreationReservation, ChatSetupFailureKind, ChatSetupPhase, ChatSetupSummary,
        ChatSummary, FilesystemIdentity, InboxSnapshot, ProjectInbox, ProjectInboxError,
    },
};

const SETUP_SCRIPT: &str = ".piu/setup.sh";
const MAX_SETUP_LOG_BYTES: usize = 256 * 1024;
const SETUP_OUTPUT_CHUNK_BYTES: usize = 8 * 1024;
const OUTPUT_TRUNCATED_MARKER: &str = "\n[Setup output truncated]\n";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatSetupChangedEvent {
    pub chat_id: String,
    pub setup: ChatSetupSummary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreatedChat {
    pub chat: ChatSummary,
    pub snapshot: InboxSnapshot,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatTerminalRequest {
    pub chat_id: String,
}

#[derive(Debug, Error)]
pub enum ChatWorkspaceError {
    #[error("the first message cannot be empty")]
    EmptyPrompt,
    #[error("fresh origin/main could not be resolved: {0}")]
    FreshMain(#[source] GitProcessError),
    #[error("could not create the managed worktree: {0}")]
    Git(#[source] GitProcessError),
    #[error(transparent)]
    Inbox(#[from] ProjectInboxError),
    #[error("could not prepare managed worktree storage: {0}")]
    WorktreeStorage(#[source] std::io::Error),
    #[error("chat setup is already running")]
    SetupAlreadyRunning,
    #[error("could not start the setup supervisor: {0}")]
    SetupSupervisor(#[source] std::io::Error),
    #[error("managed worktree ownership is invalid")]
    InvalidOwnership,
    #[error("could not reconcile an interrupted chat creation: {0}")]
    Reconciliation(String),
    #[error("chat creation was interrupted at {0}")]
    Interrupted(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CreationCheckpoint {
    Reserved,
    WorktreeCreated,
    BranchAttached,
    ChatCommitted,
}

trait CreationObserver: Send + Sync {
    fn reached(&self, _checkpoint: CreationCheckpoint) -> Result<(), ChatWorkspaceError> {
        Ok(())
    }
}

struct NoopCreationObserver;
impl CreationObserver for NoopCreationObserver {}

struct ActiveSetup {
    cancelled: Arc<AtomicBool>,
}

struct SetupRun {
    chat_id: String,
    script: PathBuf,
    project_root: PathBuf,
    worktree: PathBuf,
    attempt: u32,
    cancelled: Arc<AtomicBool>,
    on_change: Arc<dyn Fn(ChatSetupChangedEvent) + Send + Sync>,
}

struct SpawnedSetup {
    child: Child,
    receiver: mpsc::Receiver<Vec<u8>>,
    readers: Vec<JoinHandle<std::io::Result<()>>>,
}

pub struct ChatWorkspaces {
    inbox: Arc<ProjectInbox>,
    git: GitProcess,
    root: PathBuf,
    creation_lock: Mutex<()>,
    active_setups: Mutex<HashMap<String, ActiveSetup>>,
    reconciled: Mutex<bool>,
    observer: Arc<dyn CreationObserver>,
}

impl ChatWorkspaces {
    pub fn new(inbox: Arc<ProjectInbox>, git: GitProcess, managed_worktree_root: PathBuf) -> Self {
        Self::with_observer(
            inbox,
            git,
            managed_worktree_root,
            Arc::new(NoopCreationObserver),
        )
    }

    fn with_observer(
        inbox: Arc<ProjectInbox>,
        git: GitProcess,
        managed_worktree_root: PathBuf,
        observer: Arc<dyn CreationObserver>,
    ) -> Self {
        Self {
            inbox,
            git,
            root: managed_worktree_root,
            creation_lock: Mutex::new(()),
            active_setups: Mutex::new(HashMap::new()),
            reconciled: Mutex::new(false),
            observer,
        }
    }

    pub fn reconcile_once(&self) -> Result<(), ChatWorkspaceError> {
        let mut reconciled = self
            .reconciled
            .lock()
            .map_err(|_| ChatWorkspaceError::Reconciliation("recovery lock poisoned".into()))?;
        if *reconciled {
            return Ok(());
        }
        let _creation = self
            .creation_lock
            .lock()
            .map_err(|_| ChatWorkspaceError::Reconciliation("creation lock poisoned".into()))?;
        for reservation in self.inbox.pending_chat_creations()? {
            self.reconcile_reservation(&reservation)?;
        }
        self.inbox.interrupt_running_setups()?;
        *reconciled = true;
        Ok(())
    }

    pub fn create_chat(
        &self,
        project_id: i64,
        prompt: &str,
    ) -> Result<CreatedChat, ChatWorkspaceError> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(ChatWorkspaceError::EmptyPrompt);
        }
        self.reconcile_once()?;
        let _creation = self
            .creation_lock
            .lock()
            .map_err(|_| ChatWorkspaceError::Reconciliation("creation lock poisoned".into()))?;
        let project = self.inbox.project_location(project_id)?;
        let base_commit = self
            .git
            .fetch_origin_main(&project.canonical_path)
            .map_err(ChatWorkspaceError::FreshMain)?;
        fs::create_dir_all(&self.root).map_err(ChatWorkspaceError::WorktreeStorage)?;
        let chat_id = self.inbox.allocate_chat_id()?;
        let short_id = &chat_id[..8];
        let title = chat_title(prompt);
        let branch_name = format!("agent/{short_id}-{}", prompt_slug(prompt));
        let worktree_path = self.root.join(&chat_id);
        let created_at_ms = current_time_ms()?;
        fs::create_dir(&worktree_path).map_err(ChatWorkspaceError::WorktreeStorage)?;
        let worktree_root = match filesystem_identity(&worktree_path) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = fs::remove_dir(&worktree_path);
                return Err(ChatWorkspaceError::WorktreeStorage(error));
            }
        };
        let mut reservation = ChatCreationReservation {
            worktree_path,
            worktree_root,
            worktree_git_dir: None,
            chat_id,
            project,
            prompt: prompt.to_owned(),
            title,
            branch_name,
            base_commit,
            created_at_ms,
            worktree_created: false,
            branch_attached: false,
        };

        if let Err(error) = self.inbox.reserve_chat_creation(&reservation) {
            let _ = fs::remove_dir(&reservation.worktree_path);
            return Err(error.into());
        }
        self.observer.reached(CreationCheckpoint::Reserved)?;
        if let Err(error) = self.git.add_detached_worktree(
            &reservation.project.canonical_path,
            &reservation.worktree_path,
            &reservation.base_commit,
        ) {
            self.reconcile_after_error(&reservation);
            return Err(ChatWorkspaceError::Git(error));
        }
        let identity = match self
            .git
            .inspect_managed_worktree(&reservation.worktree_path)
        {
            Ok(identity) => identity,
            Err(error) => {
                self.reconcile_after_error(&reservation);
                return Err(ChatWorkspaceError::Git(error));
            }
        };
        let git_dir_identity = match filesystem_identity(&identity.git_dir) {
            Ok(identity) => identity,
            Err(error) => {
                self.reconcile_after_error(&reservation);
                return Err(ChatWorkspaceError::WorktreeStorage(error));
            }
        };
        reservation.worktree_git_dir = Some(git_dir_identity.clone());
        if let Err(error) = self
            .inbox
            .mark_creation_worktree_created(&reservation.chat_id, &git_dir_identity)
        {
            self.reconcile_after_error(&reservation);
            return Err(error.into());
        }
        reservation.worktree_created = true;
        self.observer.reached(CreationCheckpoint::WorktreeCreated)?;
        if let Err(error) = self
            .git
            .attach_new_branch(&reservation.worktree_path, &reservation.branch_name)
        {
            self.reconcile_after_error(&reservation);
            return Err(ChatWorkspaceError::Git(error));
        }
        if let Err(error) = self
            .inbox
            .mark_creation_branch_attached(&reservation.chat_id)
        {
            self.reconcile_after_error(&reservation);
            return Err(error.into());
        }
        self.observer.reached(CreationCheckpoint::BranchAttached)?;
        if let Err(error) = self.inbox.commit_chat_creation(&reservation) {
            self.reconcile_after_error(&reservation);
            return Err(error.into());
        }
        self.observer.reached(CreationCheckpoint::ChatCommitted)?;

        let snapshot = self.inbox.snapshot()?;
        let chat = snapshot
            .chats
            .iter()
            .find(|chat| chat.id == reservation.chat_id)
            .cloned()
            .ok_or_else(|| ProjectInboxError::ChatNotFound {
                chat_id: reservation.chat_id,
            })?;
        Ok(CreatedChat { chat, snapshot })
    }

    pub fn start_setup(
        self: &Arc<Self>,
        chat_id: &str,
        on_change: Arc<dyn Fn(ChatSetupChangedEvent) + Send + Sync>,
    ) -> Result<ChatSetupSummary, ChatWorkspaceError> {
        let worktree = self.inbox.chat_worktree(chat_id)?;
        let script = worktree.join(SETUP_SCRIPT);
        let script_exists = match script.try_exists() {
            Ok(exists) => exists,
            Err(_) => {
                return self.fail_setup_before_launch(
                    chat_id,
                    ChatSetupFailureKind::Launch,
                    &on_change,
                );
            }
        };
        if !script_exists {
            let setup = self.inbox.mark_setup_not_required(chat_id)?;
            on_change(setup_event(chat_id, &setup));
            return Ok(setup);
        }
        let metadata = match fs::metadata(&script) {
            Ok(metadata) => metadata,
            Err(_) => {
                return self.fail_setup_before_launch(
                    chat_id,
                    ChatSetupFailureKind::Launch,
                    &on_change,
                );
            }
        };
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return self.fail_setup_before_launch(
                chat_id,
                ChatSetupFailureKind::NotExecutable,
                &on_change,
            );
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let project_root = self.project_root_for_chat(chat_id)?;
        let setup = {
            let mut active = self
                .active_setups
                .lock()
                .map_err(|_| ChatWorkspaceError::SetupAlreadyRunning)?;
            if active.contains_key(chat_id) {
                return Err(ChatWorkspaceError::SetupAlreadyRunning);
            }
            let setup = self.inbox.begin_setup(chat_id)?;
            active.insert(
                chat_id.to_owned(),
                ActiveSetup {
                    cancelled: Arc::clone(&cancelled),
                },
            );
            setup
        };
        on_change(setup_event(chat_id, &setup));
        let manager = Arc::clone(self);
        let owned_chat_id = chat_id.to_owned();
        let thread_on_change = Arc::clone(&on_change);
        let spawn_result = thread::Builder::new()
            .name(format!("setup-{}", &chat_id[..chat_id.len().min(8)]))
            .spawn(move || {
                manager.run_setup(SetupRun {
                    chat_id: owned_chat_id,
                    script,
                    project_root,
                    worktree,
                    attempt: setup.attempt,
                    cancelled,
                    on_change: thread_on_change,
                });
            });
        if let Err(error) = spawn_result {
            self.remove_active_setup(chat_id);
            let failed = self.inbox.finish_setup(
                chat_id,
                ChatSetupPhase::Failed,
                Some(ChatSetupFailureKind::Launch),
                None,
                None,
            )?;
            on_change(setup_event(chat_id, &failed));
            return Err(ChatWorkspaceError::SetupSupervisor(error));
        }
        Ok(setup)
    }

    pub fn cancel_setup(&self, chat_id: &str) -> Result<(), ChatWorkspaceError> {
        let active = self
            .active_setups
            .lock()
            .map_err(|_| ChatWorkspaceError::SetupAlreadyRunning)?;
        let setup = active
            .get(chat_id)
            .ok_or(ChatWorkspaceError::SetupAlreadyRunning)?;
        setup.cancelled.store(true, Ordering::Release);
        Ok(())
    }

    pub fn terminal_request(
        &self,
        chat_id: &str,
    ) -> Result<ChatTerminalRequest, ChatWorkspaceError> {
        let worktree = self.inbox.chat_worktree(chat_id)?;
        if !worktree.is_dir() || worktree.parent() != Some(self.root.as_path()) {
            return Err(ChatWorkspaceError::InvalidOwnership);
        }
        Ok(ChatTerminalRequest {
            chat_id: chat_id.to_owned(),
        })
    }

    fn project_root_for_chat(&self, chat_id: &str) -> Result<PathBuf, ChatWorkspaceError> {
        self.inbox.chat_project_root(chat_id).map_err(Into::into)
    }

    fn fail_setup_before_launch(
        &self,
        chat_id: &str,
        failure: ChatSetupFailureKind,
        on_change: &Arc<dyn Fn(ChatSetupChangedEvent) + Send + Sync>,
    ) -> Result<ChatSetupSummary, ChatWorkspaceError> {
        let setup = self.inbox.begin_setup(chat_id)?;
        on_change(setup_event(chat_id, &setup));
        let setup =
            self.inbox
                .finish_setup(chat_id, ChatSetupPhase::Failed, Some(failure), None, None)?;
        on_change(setup_event(chat_id, &setup));
        Ok(setup)
    }

    fn run_setup(&self, run: SetupRun) {
        let outcome = spawn_setup_process(&run.script, &run.project_root, &run.worktree);
        let final_setup = match outcome {
            Err(_) => self.inbox.finish_setup(
                &run.chat_id,
                ChatSetupPhase::Failed,
                Some(ChatSetupFailureKind::Launch),
                None,
                None,
            ),
            Ok(mut process) => {
                let supervised = self.supervise_setup(
                    &run.chat_id,
                    run.attempt,
                    &mut process.child,
                    process.receiver,
                    process.readers,
                    &run.cancelled,
                    &run.on_change,
                );
                if supervised.is_err() {
                    terminate_process_group(&mut process.child);
                    let _ = process.child.wait();
                }
                match supervised {
                    Ok(status) if run.cancelled.load(Ordering::Acquire) => self.inbox.finish_setup(
                        &run.chat_id,
                        ChatSetupPhase::Cancelled,
                        None,
                        status.code(),
                        exit_signal(&status),
                    ),
                    Ok(status) if status.success() => self.inbox.finish_setup(
                        &run.chat_id,
                        ChatSetupPhase::Succeeded,
                        None,
                        status.code(),
                        None,
                    ),
                    Ok(status) if exit_signal(&status).is_some() => self.inbox.finish_setup(
                        &run.chat_id,
                        ChatSetupPhase::Failed,
                        Some(ChatSetupFailureKind::Signal),
                        None,
                        exit_signal(&status),
                    ),
                    Ok(status) => self.inbox.finish_setup(
                        &run.chat_id,
                        ChatSetupPhase::Failed,
                        Some(ChatSetupFailureKind::Exit),
                        status.code(),
                        None,
                    ),
                    Err(_) => self.inbox.finish_setup(
                        &run.chat_id,
                        ChatSetupPhase::Failed,
                        Some(ChatSetupFailureKind::Storage),
                        None,
                        None,
                    ),
                }
            }
        };
        if let Ok(setup) = final_setup {
            (run.on_change)(setup_event(&run.chat_id, &setup));
        }
        self.remove_active_setup(&run.chat_id);
    }

    #[allow(clippy::too_many_arguments)]
    fn supervise_setup(
        &self,
        chat_id: &str,
        attempt: u32,
        child: &mut Child,
        receiver: mpsc::Receiver<Vec<u8>>,
        readers: Vec<JoinHandle<std::io::Result<()>>>,
        cancelled: &AtomicBool,
        on_change: &Arc<dyn Fn(ChatSetupChangedEvent) + Send + Sync>,
    ) -> Result<ExitStatus, ProjectInboxError> {
        let mut retained_bytes = 0_usize;
        let mut truncated = false;
        let status = (|| {
            loop {
                while let Ok(chunk) = receiver.try_recv() {
                    self.store_setup_chunk(
                        chat_id,
                        attempt,
                        &chunk,
                        &mut retained_bytes,
                        &mut truncated,
                        on_change,
                    )?;
                }
                if cancelled.load(Ordering::Acquire) {
                    terminate_process_group(child);
                }
                if let Some(status) = child.try_wait().map_err(|error| {
                    ProjectInboxError::AppData(std::io::Error::other(format!(
                        "could not supervise setup: {error}"
                    )))
                })? {
                    break Ok(status);
                }
                match receiver.recv_timeout(Duration::from_millis(12)) {
                    Ok(chunk) => self.store_setup_chunk(
                        chat_id,
                        attempt,
                        &chunk,
                        &mut retained_bytes,
                        &mut truncated,
                        on_change,
                    )?,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {}
                }
            }
        })();
        let status = match status {
            Ok(status) => {
                terminate_process_group(child);
                status
            }
            Err(error) => {
                terminate_process_group(child);
                let _ = child.wait();
                drop(receiver);
                let _ = join_setup_readers(readers);
                return Err(error);
            }
        };
        drain_setup_output(receiver, readers, |chunk| {
            self.store_setup_chunk(
                chat_id,
                attempt,
                &chunk,
                &mut retained_bytes,
                &mut truncated,
                on_change,
            )
        })?;
        Ok(status)
    }

    fn store_setup_chunk(
        &self,
        chat_id: &str,
        attempt: u32,
        bytes: &[u8],
        retained_bytes: &mut usize,
        truncated: &mut bool,
        on_change: &Arc<dyn Fn(ChatSetupChangedEvent) + Send + Sync>,
    ) -> Result<(), ProjectInboxError> {
        let remaining = MAX_SETUP_LOG_BYTES.saturating_sub(*retained_bytes);
        if remaining > 0 {
            let accepted = bytes.len().min(remaining);
            let text = String::from_utf8_lossy(&bytes[..accepted]);
            let setup = self.inbox.append_setup_log(chat_id, attempt, &text)?;
            *retained_bytes += accepted;
            on_change(setup_event(chat_id, &setup));
        }
        if bytes.len() > remaining && !*truncated {
            let setup = self
                .inbox
                .append_setup_log(chat_id, attempt, OUTPUT_TRUNCATED_MARKER)?;
            on_change(setup_event(chat_id, &setup));
            *truncated = true;
        }
        Ok(())
    }

    fn reconcile_after_error(&self, reservation: &ChatCreationReservation) {
        let _ = self.reconcile_reservation(reservation);
    }

    fn reconcile_reservation(
        &self,
        reservation: &ChatCreationReservation,
    ) -> Result<(), ChatWorkspaceError> {
        if reservation.worktree_path != self.root.join(&reservation.chat_id) {
            return Err(ChatWorkspaceError::InvalidOwnership);
        }
        if !reservation.worktree_path.exists() {
            return Err(ChatWorkspaceError::InvalidOwnership);
        }
        let actual_root = filesystem_identity(&reservation.worktree_path)
            .map_err(|_| ChatWorkspaceError::InvalidOwnership)?;
        if actual_root.path != reservation.worktree_root.path
            || actual_root.device != reservation.worktree_root.device
            || actual_root.inode != reservation.worktree_root.inode
        {
            return Err(ChatWorkspaceError::InvalidOwnership);
        }
        let attached_at_owned_path = if reservation.worktree_path.exists() {
            self.git
                .current_branch(&reservation.worktree_path)
                .ok()
                .flatten()
                .is_some_and(|branch| branch == reservation.branch_name)
        } else {
            false
        };
        let identity = match self
            .git
            .inspect_managed_worktree(&reservation.worktree_path)
        {
            Ok(identity) => Some(identity),
            Err(_) if !reservation.worktree_created => None,
            Err(_) => return Err(ChatWorkspaceError::InvalidOwnership),
        };
        if let Some(identity) = identity {
            let expected_root = reservation
                .worktree_path
                .canonicalize()
                .map_err(|_| ChatWorkspaceError::InvalidOwnership)?;
            let expected_git_dir = reservation
                .project
                .git_dir_path
                .canonicalize()
                .map_err(|_| ChatWorkspaceError::InvalidOwnership)?;
            let actual_git_root = identity
                .root
                .canonicalize()
                .map_err(|_| ChatWorkspaceError::InvalidOwnership)?;
            let actual_git_dir = identity
                .common_git_dir
                .canonicalize()
                .map_err(|_| ChatWorkspaceError::InvalidOwnership)?;
            if actual_git_root != expected_root
                || actual_git_dir != expected_git_dir
                || identity.head != reservation.base_commit
            {
                return Err(ChatWorkspaceError::InvalidOwnership);
            }
            if reservation.worktree_created {
                let expected_worktree_git_dir = reservation
                    .worktree_git_dir
                    .as_ref()
                    .ok_or(ChatWorkspaceError::InvalidOwnership)?;
                let actual_worktree_git_dir = filesystem_identity(&identity.git_dir)
                    .map_err(|_| ChatWorkspaceError::InvalidOwnership)?;
                if actual_worktree_git_dir.path != expected_worktree_git_dir.path
                    || actual_worktree_git_dir.device != expected_worktree_git_dir.device
                    || actual_worktree_git_dir.inode != expected_worktree_git_dir.inode
                {
                    return Err(ChatWorkspaceError::InvalidOwnership);
                }
            }
            if !self
                .git
                .worktree_is_pristine(&reservation.worktree_path)
                .map_err(|error| ChatWorkspaceError::Reconciliation(error.to_string()))?
            {
                return Err(ChatWorkspaceError::InvalidOwnership);
            }
            self.git
                .remove_worktree(
                    &reservation.project.canonical_path,
                    &reservation.worktree_path,
                )
                .map_err(|error| ChatWorkspaceError::Reconciliation(error.to_string()))?;
        } else {
            fs::remove_dir(&reservation.worktree_path)
                .map_err(|error| ChatWorkspaceError::Reconciliation(error.to_string()))?;
        }
        let remove_owned_branch = (reservation.branch_attached || attached_at_owned_path)
            && self
                .git
                .branch_exists(
                    &reservation.project.canonical_path,
                    &reservation.branch_name,
                )
                .map_err(|error| ChatWorkspaceError::Reconciliation(error.to_string()))?;
        if remove_owned_branch {
            self.git
                .delete_branch(
                    &reservation.project.canonical_path,
                    &reservation.branch_name,
                )
                .map_err(|error| ChatWorkspaceError::Reconciliation(error.to_string()))?;
        }
        self.inbox.discard_chat_creation(&reservation.chat_id)?;
        Ok(())
    }

    fn remove_active_setup(&self, chat_id: &str) {
        if let Ok(mut active) = self.active_setups.lock() {
            active.remove(chat_id);
        }
    }
}

fn filesystem_identity(path: &Path) -> std::io::Result<FilesystemIdentity> {
    let path = path.canonicalize()?;
    let metadata = fs::metadata(&path)?;
    Ok(FilesystemIdentity {
        path,
        device: metadata.dev().to_string(),
        inode: metadata.ino().to_string(),
    })
}

fn setup_event(chat_id: &str, setup: &ChatSetupSummary) -> ChatSetupChangedEvent {
    ChatSetupChangedEvent {
        chat_id: chat_id.to_owned(),
        setup: setup.clone(),
    }
}

fn spawn_setup_process(
    script: &Path,
    project_root: &Path,
    worktree: &Path,
) -> std::io::Result<SpawnedSetup> {
    let mut command = Command::new(script);
    command
        .current_dir(worktree)
        .env("PIU_PROJECT_ROOT", project_root)
        .env("PIU_WORKTREE_ROOT", worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn()?;
    let (sender, receiver) = mpsc::sync_channel(8);
    let mut readers = Vec::with_capacity(2);
    if let Some(stdout) = child.stdout.take() {
        readers.push(stream_output(stdout, sender.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(stream_output(stderr, sender));
    }
    Ok(SpawnedSetup {
        child,
        receiver,
        readers,
    })
}

fn stream_output(
    mut pipe: impl Read + Send + 'static,
    sender: mpsc::SyncSender<Vec<u8>>,
) -> JoinHandle<std::io::Result<()>> {
    thread::spawn(move || {
        let mut buffer = [0_u8; SETUP_OUTPUT_CHUNK_BYTES];
        loop {
            let read = pipe.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            if sender.send(buffer[..read].to_vec()).is_err() {
                return Ok(());
            }
        }
    })
}

fn drain_setup_output(
    receiver: mpsc::Receiver<Vec<u8>>,
    readers: Vec<JoinHandle<std::io::Result<()>>>,
    mut consume: impl FnMut(Vec<u8>) -> Result<(), ProjectInboxError>,
) -> Result<(), ProjectInboxError> {
    let mut result = Ok(());
    while result.is_ok() {
        match receiver.recv() {
            Ok(chunk) => result = consume(chunk),
            Err(_) => break,
        }
    }
    drop(receiver);
    let reader_result = join_setup_readers(readers);
    result.and(reader_result)
}

fn join_setup_readers(
    readers: Vec<JoinHandle<std::io::Result<()>>>,
) -> Result<(), ProjectInboxError> {
    let mut result = Ok(());
    for reader in readers {
        let joined = match reader.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(ProjectInboxError::AppData(error)),
            Err(_) => Err(ProjectInboxError::AppData(std::io::Error::other(
                "setup output reader panicked",
            ))),
        };
        if result.is_ok() {
            result = joined;
        }
    }
    result
}

fn terminate_process_group(child: &mut Child) {
    let killed_group = child
        .id()
        .try_into()
        .ok()
        .and_then(Pid::from_raw)
        .is_some_and(|pid| kill_process_group(pid, Signal::KILL).is_ok());
    if !killed_group {
        let _ = child.kill();
    }
}

#[cfg(unix)]
fn exit_signal(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

fn current_time_ms() -> Result<i64, ChatWorkspaceError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ChatWorkspaceError::Inbox(ProjectInboxError::SystemClock))?
        .as_millis()
        .try_into()
        .map_err(|_| ChatWorkspaceError::Inbox(ProjectInboxError::SystemClock))
}

fn chat_title(prompt: &str) -> String {
    let collapsed = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = collapsed.chars().take(72).collect::<String>();
    if collapsed.chars().count() > 72 {
        title.push('…');
    }
    title
}

fn prompt_slug(prompt: &str) -> String {
    let mut slug = String::new();
    let mut needs_separator = false;
    for character in prompt.chars() {
        if character.is_ascii_alphanumeric() {
            if needs_separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            needs_separator = false;
        } else {
            needs_separator = true;
        }
        if slug.len() >= 40 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "chat".to_owned()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        process::{Command, Output},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use tempfile::TempDir;

    use super::*;
    use crate::project_inbox::ProjectInbox;

    struct RemoteFixture {
        _root: TempDir,
        working: PathBuf,
        remote: PathBuf,
    }

    impl RemoteFixture {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temporary Git fixture");
            let working = root.path().join("working");
            let remote = root.path().join("remote.git");
            run(Command::new("/usr/bin/git")
                .args(["init", "--bare", "--initial-branch=main"])
                .arg(&remote));
            run(Command::new("/usr/bin/git")
                .args(["init", "--initial-branch=main"])
                .arg(&working));
            let fixture = Self {
                _root: root,
                working,
                remote,
            };
            fixture.git(["config", "user.name", "Più Test"]);
            fixture.git(["config", "user.email", "piu-test@example.invalid"]);
            fixture.git(["config", "commit.gpgSign", "false"]);
            run(fixture
                .git_command()
                .args(["remote", "add", "origin"])
                .arg(&fixture.remote));
            fixture.write_and_push("README.md", "first\n", "initial");
            fixture
        }

        fn git<I, S>(&self, arguments: I) -> String
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            run(self.git_command().args(arguments))
        }

        fn git_command(&self) -> Command {
            let mut command = Command::new("/usr/bin/git");
            command.arg("-C").arg(&self.working);
            command
        }

        fn write_and_push(&self, relative: &str, contents: &str, message: &str) -> String {
            let path = self.working.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
            self.git(["add", relative]);
            self.git(["commit", "-m", message]);
            self.git(["push", "-u", "origin", "main"]);
            self.git(["rev-parse", "HEAD"]).trim().to_owned()
        }

        fn install_setup(&self, contents: &str, executable: bool) {
            let path = self.working.join(SETUP_SCRIPT);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            let mode = if executable { 0o755 } else { 0o644 };
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            self.git(["add", SETUP_SCRIPT]);
            self.git(["commit", "-m", "add setup"]);
            self.git(["push", "origin", "main"]);
        }
    }

    fn run(command: &mut Command) -> String {
        let Output {
            status,
            stdout,
            stderr,
        } = command.output().expect("fixture command should run");
        assert!(
            status.success(),
            "fixture command failed: {}",
            String::from_utf8_lossy(&stderr)
        );
        String::from_utf8(stdout).expect("fixture output should be UTF-8")
    }

    struct WorkspaceFixture {
        _app_data: TempDir,
        remote: RemoteFixture,
        inbox: Arc<ProjectInbox>,
        manager: Arc<ChatWorkspaces>,
        project_id: i64,
        worktrees: PathBuf,
    }

    impl WorkspaceFixture {
        fn new(remote: RemoteFixture) -> Self {
            let app_data = tempfile::tempdir().expect("temporary app data");
            let git = GitProcess::with_executable("/usr/bin/git".into());
            let inbox = Arc::new(
                ProjectInbox::with_git(&app_data.path().join("piu.sqlite3"), git.clone())
                    .expect("inbox should open"),
            );
            let project_id = inbox
                .open_repository(&remote.working)
                .expect("repository should open")
                .project
                .id;
            let worktrees = app_data.path().join("worktrees");
            let manager = Arc::new(ChatWorkspaces::new(
                Arc::clone(&inbox),
                git,
                worktrees.clone(),
            ));
            Self {
                _app_data: app_data,
                remote,
                inbox,
                manager,
                project_id,
                worktrees,
            }
        }

        fn create(&self, prompt: &str) -> CreatedChat {
            self.manager
                .create_chat(self.project_id, prompt)
                .expect("chat should be created")
        }

        fn setup_to_completion(&self, chat_id: &str) -> Vec<ChatSetupChangedEvent> {
            let (sender, receiver) = mpsc::channel();
            let sink = Arc::new(move |event| {
                sender.send(event).unwrap();
            });
            let initial = self
                .manager
                .start_setup(chat_id, sink)
                .expect("setup should start");
            if initial.phase != ChatSetupPhase::Running {
                return receiver.try_iter().collect();
            }
            let mut events = Vec::new();
            loop {
                let event = receiver
                    .recv_timeout(Duration::from_secs(5))
                    .expect("setup should publish completion");
                let finished = event.setup.phase != ChatSetupPhase::Running;
                events.push(event);
                if finished {
                    return events;
                }
            }
        }
    }

    #[test]
    fn creation_uses_fresh_remote_main_and_persists_the_message_after_worktree_ownership() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        fixture
            .manager
            .git
            .fetch_origin_main(&fixture.remote.working)
            .unwrap();
        let publisher = tempfile::tempdir().unwrap();
        run(Command::new("/usr/bin/git")
            .arg("clone")
            .arg(&fixture.remote.remote)
            .arg(publisher.path()));
        run(Command::new("/usr/bin/git")
            .args(["-C"])
            .arg(publisher.path())
            .args(["config", "user.name", "Publisher"]));
        run(Command::new("/usr/bin/git")
            .args(["-C"])
            .arg(publisher.path())
            .args(["config", "user.email", "publisher@example.invalid"]));
        fs::write(publisher.path().join("fresh.txt"), "fresh\n").unwrap();
        run(Command::new("/usr/bin/git")
            .args(["-C"])
            .arg(publisher.path())
            .args(["add", "fresh.txt"]));
        run(Command::new("/usr/bin/git")
            .args(["-C"])
            .arg(publisher.path())
            .args(["commit", "-m", "fresh remote commit"]));
        run(Command::new("/usr/bin/git")
            .args(["-C"])
            .arg(publisher.path())
            .args(["push", "origin", "main"]));
        let remote_head = run(Command::new("/usr/bin/git")
            .arg("--git-dir")
            .arg(&fixture.remote.remote)
            .args(["rev-parse", "refs/heads/main"]));

        let created = fixture.create("Repair parser ownership now");
        let worktree = fixture.worktrees.join(&created.chat.id);
        let worktree_head = run(Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&worktree)
            .args(["rev-parse", "HEAD"]));

        assert_eq!(worktree_head.trim(), remote_head.trim());
        assert!(worktree.join("fresh.txt").is_file());
        assert_eq!(
            created.chat.branch_name,
            format!(
                "agent/{}-repair-parser-ownership-now",
                &created.chat.id[..8]
            )
        );
        assert_eq!(
            fixture.inbox.first_user_message(&created.chat.id).unwrap(),
            "Repair parser ownership now"
        );
        assert!(created.snapshot.drafts.is_empty());
    }

    #[test]
    fn failed_fetch_never_falls_back_to_a_cached_remote_main() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        fixture
            .manager
            .git
            .fetch_origin_main(&fixture.remote.working)
            .unwrap();
        fs::rename(
            &fixture.remote.remote,
            fixture.remote.remote.with_extension("offline"),
        )
        .unwrap();

        let error = fixture
            .manager
            .create_chat(fixture.project_id, "Do not use cached state")
            .expect_err("fresh fetch should be mandatory");

        assert!(matches!(error, ChatWorkspaceError::FreshMain(_)));
        assert!(fixture.inbox.snapshot().unwrap().chats.is_empty());
        assert!(!fixture.worktrees.exists());
    }

    #[test]
    fn creation_refuses_a_repository_replaced_after_it_was_opened() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let original_repository = fixture.remote.working.with_extension("opened-repository");
        fs::rename(&fixture.remote.working, &original_repository).unwrap();
        run(Command::new("/usr/bin/git")
            .arg("clone")
            .arg(&fixture.remote.remote)
            .arg(&fixture.remote.working));

        let error = fixture
            .manager
            .create_chat(fixture.project_id, "Never mutate a replacement repository")
            .expect_err("the stored repository identity must be revalidated");

        assert!(matches!(
            error,
            ChatWorkspaceError::Inbox(ProjectInboxError::InvalidRepository)
        ));
        assert!(!fixture.worktrees.exists());
        assert!(fixture.inbox.snapshot().unwrap().chats.is_empty());
    }

    #[test]
    fn concurrent_first_sends_receive_isolated_branches_and_worktrees() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let manager = Arc::clone(&fixture.manager);
        let first_project = fixture.project_id;
        let first = thread::spawn(move || manager.create_chat(first_project, "First parser fix"));
        let manager = Arc::clone(&fixture.manager);
        let second_project = fixture.project_id;
        let second =
            thread::spawn(move || manager.create_chat(second_project, "Second parser fix"));
        let first = first.join().unwrap().unwrap().chat;
        let second = second.join().unwrap().unwrap().chat;

        assert_ne!(first.id, second.id);
        assert_ne!(first.branch_name, second.branch_name);
        assert!(fixture.worktrees.join(first.id).is_dir());
        assert!(fixture.worktrees.join(second.id).is_dir());
        assert_eq!(fixture.inbox.snapshot().unwrap().chats.len(), 2);
    }

    struct FailAt {
        checkpoint: CreationCheckpoint,
    }

    impl CreationObserver for FailAt {
        fn reached(&self, checkpoint: CreationCheckpoint) -> Result<(), ChatWorkspaceError> {
            if checkpoint == self.checkpoint {
                Err(ChatWorkspaceError::Interrupted(match checkpoint {
                    CreationCheckpoint::Reserved => "reservation",
                    CreationCheckpoint::WorktreeCreated => "worktree",
                    CreationCheckpoint::BranchAttached => "branch",
                    CreationCheckpoint::ChatCommitted => "chat",
                }))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn relaunch_reconciles_every_incomplete_durable_creation_step() {
        for checkpoint in [
            CreationCheckpoint::Reserved,
            CreationCheckpoint::WorktreeCreated,
            CreationCheckpoint::BranchAttached,
        ] {
            let fixture = WorkspaceFixture::new(RemoteFixture::new());
            let crashing = ChatWorkspaces::with_observer(
                Arc::clone(&fixture.inbox),
                GitProcess::with_executable("/usr/bin/git".into()),
                fixture.worktrees.clone(),
                Arc::new(FailAt { checkpoint }),
            );
            crashing
                .create_chat(fixture.project_id, "Recover owned state")
                .expect_err("checkpoint should simulate a crash");
            let reservation = fixture.inbox.pending_chat_creations().unwrap().remove(0);

            let relaunched = ChatWorkspaces::new(
                Arc::clone(&fixture.inbox),
                GitProcess::with_executable("/usr/bin/git".into()),
                fixture.worktrees.clone(),
            );
            relaunched.reconcile_once().unwrap();

            assert!(fixture.inbox.pending_chat_creations().unwrap().is_empty());
            assert!(!reservation.worktree_path.exists());
            assert!(
                !relaunched
                    .git
                    .branch_exists(
                        &reservation.project.canonical_path,
                        &reservation.branch_name
                    )
                    .unwrap()
            );
            assert!(fixture.inbox.snapshot().unwrap().chats.is_empty());
        }
    }

    #[test]
    fn a_committed_chat_survives_a_crash_after_the_database_transaction() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let crashing = ChatWorkspaces::with_observer(
            Arc::clone(&fixture.inbox),
            GitProcess::with_executable("/usr/bin/git".into()),
            fixture.worktrees.clone(),
            Arc::new(FailAt {
                checkpoint: CreationCheckpoint::ChatCommitted,
            }),
        );
        crashing
            .create_chat(fixture.project_id, "Keep committed chat")
            .expect_err("checkpoint should simulate a crash");

        let relaunched = ChatWorkspaces::new(
            Arc::clone(&fixture.inbox),
            GitProcess::with_executable("/usr/bin/git".into()),
            fixture.worktrees.clone(),
        );
        relaunched.reconcile_once().unwrap();
        let snapshot = fixture.inbox.snapshot().unwrap();

        assert_eq!(snapshot.chats.len(), 1);
        assert!(fixture.worktrees.join(&snapshot.chats[0].id).is_dir());
        assert_eq!(
            fixture
                .inbox
                .first_user_message(&snapshot.chats[0].id)
                .unwrap(),
            "Keep committed chat"
        );
    }

    #[test]
    fn recovery_never_deletes_a_colliding_branch_it_did_not_create() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let crashing = ChatWorkspaces::with_observer(
            Arc::clone(&fixture.inbox),
            GitProcess::with_executable("/usr/bin/git".into()),
            fixture.worktrees.clone(),
            Arc::new(FailAt {
                checkpoint: CreationCheckpoint::Reserved,
            }),
        );
        crashing
            .create_chat(fixture.project_id, "Protect user branch")
            .expect_err("checkpoint should leave a reservation");
        let reservation = fixture.inbox.pending_chat_creations().unwrap().remove(0);
        fixture
            .remote
            .git(["branch", &reservation.branch_name, &reservation.base_commit]);

        fixture.manager.reconcile_once().unwrap();

        assert!(
            fixture
                .manager
                .git
                .branch_exists(
                    &reservation.project.canonical_path,
                    &reservation.branch_name
                )
                .unwrap()
        );
    }

    #[test]
    fn recovery_fails_closed_when_the_managed_path_was_replaced() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let crashing = ChatWorkspaces::with_observer(
            Arc::clone(&fixture.inbox),
            GitProcess::with_executable("/usr/bin/git".into()),
            fixture.worktrees.clone(),
            Arc::new(FailAt {
                checkpoint: CreationCheckpoint::WorktreeCreated,
            }),
        );
        crashing
            .create_chat(fixture.project_id, "Keep replacement data")
            .expect_err("checkpoint should leave a worktree");
        let reservation = fixture.inbox.pending_chat_creations().unwrap().remove(0);
        let moved_owned_worktree = fixture.worktrees.join("interrupted-owned-worktree");
        fs::rename(&reservation.worktree_path, &moved_owned_worktree).unwrap();
        fs::create_dir(&reservation.worktree_path).unwrap();
        fs::write(
            reservation.worktree_path.join("user-data.txt"),
            "do not delete\n",
        )
        .unwrap();

        let error = fixture
            .manager
            .reconcile_once()
            .expect_err("an unprovable path must remain untouched");

        assert!(matches!(error, ChatWorkspaceError::InvalidOwnership));
        assert_eq!(
            fs::read_to_string(reservation.worktree_path.join("user-data.txt")).unwrap(),
            "do not delete\n"
        );
        assert_eq!(fixture.inbox.pending_chat_creations().unwrap().len(), 1);
    }

    #[test]
    fn recovery_preserves_untracked_data_in_an_interrupted_owned_worktree() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let crashing = ChatWorkspaces::with_observer(
            Arc::clone(&fixture.inbox),
            GitProcess::with_executable("/usr/bin/git".into()),
            fixture.worktrees.clone(),
            Arc::new(FailAt {
                checkpoint: CreationCheckpoint::WorktreeCreated,
            }),
        );
        crashing
            .create_chat(fixture.project_id, "Protect untracked recovery data")
            .expect_err("checkpoint should leave an owned worktree");
        let reservation = fixture.inbox.pending_chat_creations().unwrap().remove(0);
        fs::write(
            reservation.worktree_path.join("user-notes.txt"),
            "never remove this file\n",
        )
        .unwrap();

        let error = fixture
            .manager
            .reconcile_once()
            .expect_err("changed worktrees must fail closed");

        assert!(matches!(error, ChatWorkspaceError::InvalidOwnership));
        assert_eq!(
            fs::read_to_string(reservation.worktree_path.join("user-notes.txt")).unwrap(),
            "never remove this file\n"
        );
        assert_eq!(fixture.inbox.pending_chat_creations().unwrap().len(), 1);
    }

    #[test]
    fn recovery_preserves_tracked_modifications_in_an_interrupted_owned_worktree() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let crashing = ChatWorkspaces::with_observer(
            Arc::clone(&fixture.inbox),
            GitProcess::with_executable("/usr/bin/git".into()),
            fixture.worktrees.clone(),
            Arc::new(FailAt {
                checkpoint: CreationCheckpoint::WorktreeCreated,
            }),
        );
        crashing
            .create_chat(fixture.project_id, "Protect tracked recovery changes")
            .expect_err("checkpoint should leave an owned worktree");
        let reservation = fixture.inbox.pending_chat_creations().unwrap().remove(0);
        fs::write(
            reservation.worktree_path.join("README.md"),
            "modified after interruption\n",
        )
        .unwrap();

        let error = fixture
            .manager
            .reconcile_once()
            .expect_err("changed worktrees must fail closed");

        assert!(matches!(error, ChatWorkspaceError::InvalidOwnership));
        assert_eq!(
            fs::read_to_string(reservation.worktree_path.join("README.md")).unwrap(),
            "modified after interruption\n"
        );
        assert_eq!(fixture.inbox.pending_chat_creations().unwrap().len(), 1);
    }

    #[test]
    fn recovery_rejects_a_same_repository_same_commit_worktree_replacement() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let crashing = ChatWorkspaces::with_observer(
            Arc::clone(&fixture.inbox),
            GitProcess::with_executable("/usr/bin/git".into()),
            fixture.worktrees.clone(),
            Arc::new(FailAt {
                checkpoint: CreationCheckpoint::WorktreeCreated,
            }),
        );
        crashing
            .create_chat(fixture.project_id, "Protect exact worktree ownership")
            .expect_err("checkpoint should leave an owned worktree");
        let reservation = fixture.inbox.pending_chat_creations().unwrap().remove(0);
        let replacement = fixture.worktrees.join("same-repository-replacement");
        run(fixture
            .remote
            .git_command()
            .args(["worktree", "add", "--detach"])
            .arg(&replacement)
            .arg(&reservation.base_commit));
        let moved_owned_worktree = fixture.worktrees.join("interrupted-owned-worktree");
        fs::rename(&reservation.worktree_path, &moved_owned_worktree).unwrap();
        fs::rename(&replacement, &reservation.worktree_path).unwrap();
        run(fixture
            .remote
            .git_command()
            .args(["worktree", "repair"])
            .arg(&reservation.worktree_path));
        fs::write(
            reservation.worktree_path.join("replacement-data.txt"),
            "same repository, different worktree\n",
        )
        .unwrap();

        let error = fixture
            .manager
            .reconcile_once()
            .expect_err("a different worktree instance must remain untouched");

        assert!(matches!(error, ChatWorkspaceError::InvalidOwnership));
        assert_eq!(
            fs::read_to_string(reservation.worktree_path.join("replacement-data.txt")).unwrap(),
            "same repository, different worktree\n"
        );
        assert_eq!(fixture.inbox.pending_chat_creations().unwrap().len(), 1);
    }

    #[test]
    fn missing_setup_is_silent_and_an_executable_script_honors_its_shebang_and_environment() {
        let missing = WorkspaceFixture::new(RemoteFixture::new());
        let chat = missing.create("No setup repository").chat;
        let events = missing.setup_to_completion(&chat.id);
        assert_eq!(
            events.last().unwrap().setup.phase,
            ChatSetupPhase::NotRequired
        );
        assert!(events.last().unwrap().setup.log.is_empty());

        let remote = RemoteFixture::new();
        remote.install_setup(
            "#!/bin/zsh\nprintf 'project=%s\\nworktree=%s\\n' \"$PIU_PROJECT_ROOT\" \"$PIU_WORKTREE_ROOT\"\n",
            true,
        );
        let fixture = WorkspaceFixture::new(remote);
        let chat = fixture.create("Run direct setup").chat;
        let events = fixture.setup_to_completion(&chat.id);
        let setup = &events.last().unwrap().setup;

        assert_eq!(setup.phase, ChatSetupPhase::Succeeded);
        assert!(setup.log.contains(&format!(
            "project={}",
            fixture.remote.working.canonicalize().unwrap().display()
        )));
        assert!(setup.log.contains(&format!(
            "worktree={}",
            fixture.worktrees.join(&chat.id).display()
        )));
    }

    #[test]
    fn setup_classifies_non_executable_exit_signal_and_cancellation_failures() {
        let non_executable_remote = RemoteFixture::new();
        non_executable_remote.install_setup("#!/bin/zsh\nexit 0\n", false);
        let non_executable = WorkspaceFixture::new(non_executable_remote);
        let chat = non_executable.create("Non executable setup").chat;
        let events = non_executable.setup_to_completion(&chat.id);
        assert_eq!(
            events.last().unwrap().setup.failure,
            Some(ChatSetupFailureKind::NotExecutable)
        );

        let exit_remote = RemoteFixture::new();
        exit_remote.install_setup("#!/bin/zsh\necho broken\nexit 23\n", true);
        let exit = WorkspaceFixture::new(exit_remote);
        let chat = exit.create("Exit setup").chat;
        let events = exit.setup_to_completion(&chat.id);
        assert_eq!(
            events.last().unwrap().setup.failure,
            Some(ChatSetupFailureKind::Exit)
        );
        assert_eq!(events.last().unwrap().setup.exit_code, Some(23));

        let signal_remote = RemoteFixture::new();
        signal_remote.install_setup("#!/bin/zsh\nkill -TERM $$\n", true);
        let signal = WorkspaceFixture::new(signal_remote);
        let chat = signal.create("Signal setup").chat;
        let events = signal.setup_to_completion(&chat.id);
        assert_eq!(
            events.last().unwrap().setup.failure,
            Some(ChatSetupFailureKind::Signal)
        );

        let cancel_remote = RemoteFixture::new();
        cancel_remote.install_setup(
            "#!/bin/zsh\necho started\nwhile true; do sleep 1; done\n",
            true,
        );
        let cancel = WorkspaceFixture::new(cancel_remote);
        let chat = cancel.create("Cancel setup").chat;
        let (sender, receiver) = mpsc::channel();
        cancel
            .manager
            .start_setup(&chat.id, Arc::new(move |event| sender.send(event).unwrap()))
            .unwrap();
        loop {
            let event = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
            if event.setup.log.contains("started") {
                break;
            }
        }
        cancel.manager.cancel_setup(&chat.id).unwrap();
        let final_event = loop {
            let event = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
            if event.setup.phase != ChatSetupPhase::Running {
                break event;
            }
        };
        assert_eq!(final_event.setup.phase, ChatSetupPhase::Cancelled);
    }

    #[test]
    fn an_unlaunchable_shebang_preserves_the_chat_as_a_retryable_setup_failure() {
        let remote = RemoteFixture::new();
        remote.install_setup("#!/no/such/piu-interpreter\necho unreachable\n", true);
        let fixture = WorkspaceFixture::new(remote);
        let chat = fixture.create("Invalid setup interpreter").chat;

        let events = fixture.setup_to_completion(&chat.id);
        let setup = &events.last().unwrap().setup;

        assert_eq!(setup.phase, ChatSetupPhase::Failed);
        assert_eq!(setup.failure, Some(ChatSetupFailureKind::Launch));
        assert!(fixture.worktrees.join(chat.id).is_dir());
    }

    #[test]
    fn large_setup_output_is_streamed_in_bounded_updates_without_blocking_completion() {
        let remote = RemoteFixture::new();
        remote.install_setup(
            "#!/bin/zsh\ni=0\nwhile (( i < 40000 )); do printf 'line-%05d-abcdefghijklmnopqrstuvwxyz\\n' $i; (( i += 1 )); done\n",
            true,
        );
        let fixture = WorkspaceFixture::new(remote);
        let chat = fixture.create("Stream large setup").chat;
        let started = Instant::now();
        let events = fixture.setup_to_completion(&chat.id);
        let final_setup = &events.last().unwrap().setup;

        assert_eq!(final_setup.phase, ChatSetupPhase::Succeeded);
        assert!(events.len() > 4);
        assert!(final_setup.log.len() <= MAX_SETUP_LOG_BYTES + OUTPUT_TRUNCATED_MARKER.len());
        assert!(final_setup.log.ends_with(OUTPUT_TRUNCATED_MARKER));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn post_exit_output_drain_unblocks_readers_when_the_bounded_channel_is_full() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let channel_is_full = Arc::new(AtomicBool::new(false));
        let reader_gate = Arc::clone(&channel_is_full);
        let reader = thread::spawn(move || -> std::io::Result<()> {
            sender.send(vec![0]).unwrap();
            sender.send(vec![1]).unwrap();
            reader_gate.store(true, Ordering::Release);
            sender.send(vec![2]).unwrap();
            sender.send(vec![3]).unwrap();
            Ok(())
        });
        while !channel_is_full.load(Ordering::Acquire) {
            thread::yield_now();
        }
        let mut drained = Vec::new();

        drain_setup_output(receiver, vec![reader], |chunk| {
            drained.extend(chunk);
            Ok(())
        })
        .unwrap();

        assert_eq!(drained, vec![0, 1, 2, 3]);
    }

    #[test]
    fn output_reader_failures_are_reported_after_every_reader_is_joined() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let healthy_reader = thread::spawn(|| Ok(()));
        let failing_reader = thread::spawn(|| {
            Err(std::io::Error::other(
                "fixture setup output could not be read",
            ))
        });

        let error = drain_setup_output(receiver, vec![healthy_reader, failing_reader], |_| Ok(()))
            .expect_err("a reader failure must fail setup supervision");

        assert!(matches!(error, ProjectInboxError::AppData(_)));
        assert!(error.to_string().contains("fixture setup output"));
    }

    #[test]
    fn failed_setup_can_be_retried_from_the_preserved_worktree_after_relaunch() {
        let remote = RemoteFixture::new();
        remote.install_setup(
            "#!/bin/zsh\nif [[ ! -f .setup-retry ]]; then touch .setup-retry; echo first-failure; exit 9; fi\necho retry-success\n",
            true,
        );
        let fixture = WorkspaceFixture::new(remote);
        let chat = fixture.create("Retry setup").chat;
        let first = fixture.setup_to_completion(&chat.id);
        assert_eq!(first.last().unwrap().setup.phase, ChatSetupPhase::Failed);

        let relaunched = Arc::new(ChatWorkspaces::new(
            Arc::clone(&fixture.inbox),
            GitProcess::with_executable("/usr/bin/git".into()),
            fixture.worktrees.clone(),
        ));
        relaunched.reconcile_once().unwrap();
        let (sender, receiver) = mpsc::channel();
        relaunched
            .start_setup(&chat.id, Arc::new(move |event| sender.send(event).unwrap()))
            .unwrap();
        let final_event = loop {
            let event = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            if event.setup.phase != ChatSetupPhase::Running {
                break event;
            }
        };

        assert_eq!(final_event.setup.phase, ChatSetupPhase::Succeeded);
        assert_eq!(final_event.setup.attempt, 2);
        assert!(final_event.setup.log.contains("retry-success"));
        assert!(!final_event.setup.log.contains("first-failure"));
    }

    #[test]
    fn terminal_requests_validate_managed_ownership_without_exposing_a_path() {
        let fixture = WorkspaceFixture::new(RemoteFixture::new());
        let chat = fixture.create("Open recovery terminal").chat;

        let request = fixture.manager.terminal_request(&chat.id).unwrap();

        assert_eq!(request.chat_id, chat.id);
    }
}

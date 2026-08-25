use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, broadcast};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::{
    chat_workspaces::{ChatWorkspaceError, ChatWorkspaces},
    pi_rpc::{PiRpcChild, PiRpcError, PiRpcPolicy, PiRpcProcessSpec},
    project_inbox::{ChatSessionReference, ChatSetupPhase, ProjectInbox, ProjectInboxError},
};

const MODEL_PROVIDER: &str = "openai-codex";
const MODEL_ID: &str = "gpt-5.6-sol";
const THINKING_LEVEL: &str = "xhigh";
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationPhase {
    Idle,
    Running,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ConversationSnapshot {
    pub failure: Option<String>,
    pub items: Vec<ConversationItem>,
    pub phase: ConversationPhase,
}

impl ConversationSnapshot {
    fn stopped() -> Self {
        Self {
            failure: None,
            items: Vec::new(),
            phase: ConversationPhase::Stopped,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationItem {
    Message {
        id: String,
        role: ConversationRole,
        text: String,
    },
    Reasoning {
        id: String,
        text: String,
    },
    Tool {
        detail: String,
        id: String,
        name: String,
        status: ConversationToolStatus,
    },
    Usage {
        #[serde(rename = "cacheReadTokens")]
        #[ts(type = "number | null")]
        cache_read_tokens: Option<u64>,
        id: String,
        #[serde(rename = "inputTokens")]
        #[ts(type = "number")]
        input_tokens: u64,
        #[serde(rename = "outputTokens")]
        #[ts(type = "number")]
        output_tokens: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationToolStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationEvent {
    ItemAdded {
        item: ConversationItem,
    },
    TextDelta {
        delta: String,
        #[serde(rename = "itemId")]
        item_id: String,
    },
    ReasoningDelta {
        delta: String,
        #[serde(rename = "itemId")]
        item_id: String,
    },
    ToolUpdate {
        detail: String,
        #[serde(rename = "itemId")]
        item_id: String,
        status: ConversationToolStatus,
    },
    UsageUpdate {
        #[serde(rename = "cacheReadTokens")]
        #[ts(type = "number | null")]
        cache_read_tokens: Option<u64>,
        #[serde(rename = "inputTokens")]
        #[ts(type = "number")]
        input_tokens: u64,
        #[serde(rename = "itemId")]
        item_id: String,
        #[serde(rename = "outputTokens")]
        #[ts(type = "number")]
        output_tokens: u64,
    },
    TurnStarted,
    TurnCompleted,
    TurnStopped,
    TurnFailed {
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatRuntimeChangedEvent {
    pub chat_id: String,
    pub event: ConversationEvent,
}

#[derive(Debug, Error)]
pub enum ChatRuntimeHostError {
    #[error("a chat message cannot be empty")]
    EmptyMessage,
    #[error("chat {chat_id} has no active Pi runtime")]
    NotActive { chat_id: String },
    #[error("chat {chat_id} cannot start before setup finishes ({phase:?})")]
    SetupIncomplete {
        chat_id: String,
        phase: ChatSetupPhase,
    },
    #[error("the chat runtime paths must be absolute")]
    NonAbsolutePath,
    #[error("HOME is unavailable or is not an absolute path")]
    InvalidHome,
    #[error("the Pi session state was invalid: {0}")]
    InvalidSessionState(String),
    #[error("could not prepare Più runtime state: {0}")]
    RuntimeStorage(#[source] std::io::Error),
    #[error(transparent)]
    Workspace(#[from] ChatWorkspaceError),
    #[error(transparent)]
    Inbox(#[from] ProjectInboxError),
    #[error(transparent)]
    Rpc(#[from] PiRpcError),
    #[error("chat runtime lock is poisoned")]
    Lock,
}

struct RuntimePaths {
    node: PathBuf,
    launcher: PathBuf,
    executable_path: OsString,
    git_exec_path: PathBuf,
    git_template_directory: PathBuf,
    agent_directory: PathBuf,
    session_directory: PathBuf,
    credential_lock_directory: PathBuf,
    app_skill_directory: PathBuf,
    home: PathBuf,
}

struct ChatSlot {
    operation: AsyncMutex<()>,
    active: Mutex<Option<ActiveChat>>,
    projection: Mutex<ConversationProjection>,
}

impl ChatSlot {
    fn new() -> Self {
        Self {
            operation: AsyncMutex::new(()),
            active: Mutex::new(None),
            projection: Mutex::new(ConversationProjection::new(
                ConversationSnapshot::stopped(),
                0,
                false,
            )),
        }
    }
}

struct ActiveChat {
    child: Arc<PiRpcChild>,
    command_generation: u64,
    stop_events: CancellationToken,
}

struct ConversationProjection {
    snapshot: ConversationSnapshot,
    hydrated: bool,
    next_message_index: usize,
    active_assistant_index: Option<usize>,
    tool_content_ids: HashMap<usize, String>,
    pending_user_items: VecDeque<(String, String)>,
}

impl ConversationProjection {
    fn new(snapshot: ConversationSnapshot, next_message_index: usize, hydrated: bool) -> Self {
        Self {
            snapshot,
            hydrated,
            next_message_index,
            active_assistant_index: None,
            tool_content_ids: HashMap::new(),
            pending_user_items: VecDeque::new(),
        }
    }
}

struct HostInner {
    inbox: Arc<ProjectInbox>,
    workspaces: Arc<ChatWorkspaces>,
    paths: RuntimePaths,
    slots: Mutex<HashMap<String, Arc<ChatSlot>>>,
    events: broadcast::Sender<ChatRuntimeChangedEvent>,
}

#[derive(Clone)]
pub struct ChatRuntimeHost {
    inner: Arc<HostInner>,
}

impl ChatRuntimeHost {
    pub fn production(
        inbox: Arc<ProjectInbox>,
        workspaces: Arc<ChatWorkspaces>,
        app_data_directory: &Path,
        resource_directory: &Path,
    ) -> Result<Self, ChatRuntimeHostError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(ChatRuntimeHostError::InvalidHome)?;
        Self::new(
            inbox,
            workspaces,
            app_data_directory,
            resource_directory,
            &home,
        )
    }

    pub fn new(
        inbox: Arc<ProjectInbox>,
        workspaces: Arc<ChatWorkspaces>,
        app_data_directory: &Path,
        resource_directory: &Path,
        home: &Path,
    ) -> Result<Self, ChatRuntimeHostError> {
        if !app_data_directory.is_absolute() || !resource_directory.is_absolute() {
            return Err(ChatRuntimeHostError::NonAbsolutePath);
        }
        if !home.is_absolute() {
            return Err(ChatRuntimeHostError::InvalidHome);
        }
        let (events, _) = broadcast::channel(2_048);
        let bundled_git_bin = resource_directory.join("git/bin");
        let executable_path = std::env::join_paths([
            bundled_git_bin.as_path(),
            Path::new("/usr/bin"),
            Path::new("/bin"),
        ])
        .expect("fixed bundled runtime paths must form a valid PATH");
        Ok(Self {
            inner: Arc::new(HostInner {
                inbox,
                workspaces,
                paths: RuntimePaths {
                    node: resource_directory.join("agent-runtime/node/bin/node"),
                    launcher: resource_directory
                        .join("agent-runtime/pi/launcher/chat-launcher.mjs"),
                    executable_path,
                    git_exec_path: resource_directory.join("git/libexec/git-core"),
                    git_template_directory: resource_directory.join("git/share/git-core/templates"),
                    agent_directory: app_data_directory.join("agent"),
                    session_directory: app_data_directory.join("sessions"),
                    credential_lock_directory: app_data_directory.join("credential-locks"),
                    app_skill_directory: resource_directory.join("agent-runtime/skills"),
                    home: home.to_path_buf(),
                },
                slots: Mutex::new(HashMap::new()),
                events,
            }),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ChatRuntimeChangedEvent> {
        self.inner.events.subscribe()
    }

    pub fn snapshot(&self, chat_id: &str) -> Result<ConversationSnapshot, ChatRuntimeHostError> {
        let slot = self.slot(chat_id)?;
        slot.projection
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)
            .map(|projection| projection.snapshot.clone())
    }

    pub fn has_active_turn(&self) -> Result<bool, ChatRuntimeHostError> {
        let slots = self
            .inner
            .slots
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?;
        for slot in slots.values() {
            if slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?
                .snapshot
                .phase
                == ConversationPhase::Running
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn send(&self, chat_id: &str, text: &str) -> Result<(), ChatRuntimeHostError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ChatRuntimeHostError::EmptyMessage);
        }
        self.open_for_send(chat_id).await?;
        let command = serde_json::json!({
            "type": "prompt",
            "message": text,
            "streamingBehavior": "steer"
        });
        let sent = self.send_active(chat_id, text, command.clone()).await;
        if !matches!(sent, Err(ChatRuntimeHostError::NotActive { .. })) {
            return sent;
        }

        self.open_for_send(chat_id).await?;
        self.send_active(chat_id, text, command).await
    }

    pub async fn steer(&self, chat_id: &str, text: &str) -> Result<(), ChatRuntimeHostError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ChatRuntimeHostError::EmptyMessage);
        }
        self.send_active(
            chat_id,
            text,
            serde_json::json!({ "type": "steer", "message": text }),
        )
        .await
    }

    pub async fn abort(&self, chat_id: &str) -> Result<(), ChatRuntimeHostError> {
        let slot = self.slot(chat_id)?;
        let _operation = slot.operation.lock().await;
        let child = slot
            .active
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .as_ref()
            .map(|active| Arc::clone(&active.child))
            .ok_or_else(|| ChatRuntimeHostError::NotActive {
                chat_id: chat_id.to_owned(),
            })?;
        child
            .request(
                serde_json::json!({ "type": "abort" }),
                CancellationToken::new(),
            )
            .await?;
        let active = {
            let mut active = slot.active.lock().map_err(|_| ChatRuntimeHostError::Lock)?;
            if active
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(&active.child, &child))
            {
                active.take()
            } else {
                None
            }
        };
        let shutdown = if let Some(active) = active {
            active.stop_events.cancel();
            active.child.shutdown().await
        } else {
            Ok(())
        };
        let changed = {
            let mut projection = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            let changed = projection.snapshot.phase != ConversationPhase::Stopped;
            projection.snapshot.failure = None;
            projection.snapshot.phase = ConversationPhase::Stopped;
            changed
        };
        if changed {
            self.emit(chat_id, ConversationEvent::TurnStopped);
        }
        shutdown.map_err(Into::into)
    }

    async fn send_active(
        &self,
        chat_id: &str,
        text: &str,
        command: Value,
    ) -> Result<(), ChatRuntimeHostError> {
        let slot = self.slot(chat_id)?;
        let _operation = slot.operation.lock().await;
        let (child, command_generation) = {
            let mut active = slot.active.lock().map_err(|_| ChatRuntimeHostError::Lock)?;
            let active = active
                .as_mut()
                .ok_or_else(|| ChatRuntimeHostError::NotActive {
                    chat_id: chat_id.to_owned(),
                })?;
            active.command_generation = active.command_generation.wrapping_add(1);
            (Arc::clone(&active.child), active.command_generation)
        };
        let expected_index = slot
            .projection
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .next_message_index;
        if let Err(error) = child.request(command, CancellationToken::new()).await {
            let mut active = slot.active.lock().map_err(|_| ChatRuntimeHostError::Lock)?;
            if let Some(active) = active.as_mut().filter(|active| {
                Arc::ptr_eq(&active.child, &child)
                    && active.command_generation == command_generation
            }) {
                active.command_generation = active.command_generation.wrapping_add(1);
            }
            return Err(error.into());
        }
        let item_id = format!("message-{expected_index}");
        let (turn_started, item_added) = {
            let mut projection = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            let turn_started = projection.snapshot.phase != ConversationPhase::Running;
            projection.snapshot.failure = None;
            projection.snapshot.phase = ConversationPhase::Running;
            let item_added = if projection
                .snapshot
                .items
                .iter()
                .any(|item| item.id() == item_id)
            {
                None
            } else {
                let item = ConversationItem::Message {
                    id: item_id,
                    role: ConversationRole::User,
                    text: text.to_owned(),
                };
                projection.next_message_index =
                    projection.next_message_index.max(expected_index + 1);
                projection
                    .pending_user_items
                    .push_back((item.id().to_owned(), text.to_owned()));
                projection.snapshot.items.push(item.clone());
                Some(item)
            };
            (turn_started, item_added)
        };
        if turn_started {
            self.emit(chat_id, ConversationEvent::TurnStarted);
        }
        if let Some(item) = item_added {
            self.emit(chat_id, ConversationEvent::ItemAdded { item });
        }
        Ok(())
    }

    pub async fn open(&self, chat_id: &str) -> Result<ConversationSnapshot, ChatRuntimeHostError> {
        self.open_slot(chat_id, false).await
    }

    async fn open_for_send(
        &self,
        chat_id: &str,
    ) -> Result<ConversationSnapshot, ChatRuntimeHostError> {
        self.open_slot(chat_id, true).await
    }

    async fn open_slot(
        &self,
        chat_id: &str,
        keep_stopped_runtime: bool,
    ) -> Result<ConversationSnapshot, ChatRuntimeHostError> {
        let slot = self.slot(chat_id)?;
        let _operation = slot.operation.lock().await;
        if slot
            .active
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .is_some()
        {
            return slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)
                .map(|projection| projection.snapshot.clone());
        }
        if !keep_stopped_runtime {
            let projection = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            if projection.hydrated {
                return Ok(projection.snapshot.clone());
            }
        }

        let owned_chat_id = chat_id.to_owned();
        let workspaces = Arc::clone(&self.inner.workspaces);
        let context =
            tokio::task::spawn_blocking(move || workspaces.agent_launch_context(&owned_chat_id))
                .await
                .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))??;
        if !matches!(
            context.setup.phase,
            ChatSetupPhase::Succeeded | ChatSetupPhase::NotRequired
        ) {
            return Err(ChatRuntimeHostError::SetupIncomplete {
                chat_id: chat_id.to_owned(),
                phase: context.setup.phase,
            });
        }

        tokio::fs::create_dir_all(&self.inner.paths.agent_directory)
            .await
            .map_err(ChatRuntimeHostError::RuntimeStorage)?;
        tokio::fs::create_dir_all(&self.inner.paths.session_directory)
            .await
            .map_err(ChatRuntimeHostError::RuntimeStorage)?;
        tokio::fs::create_dir_all(&self.inner.paths.credential_lock_directory)
            .await
            .map_err(ChatRuntimeHostError::RuntimeStorage)?;

        let inbox = Arc::clone(&self.inner.inbox);
        let owned_chat_id = chat_id.to_owned();
        let stored_session =
            tokio::task::spawn_blocking(move || inbox.chat_session(&owned_chat_id))
                .await
                .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))??;
        if stored_session
            .as_ref()
            .is_some_and(|session| !self.is_owned_session_path(&session.path))
        {
            return Err(ChatRuntimeHostError::InvalidSessionState(
                "the stored Pi session path was outside application storage".into(),
            ));
        }
        let spec = self.process_spec(
            &context.worktree_path,
            stored_session
                .as_ref()
                .map(|session| session.path.as_path()),
        );
        let child = Arc::new(PiRpcChild::launch(spec, PiRpcPolicy::default()).await?);
        let opened = self.inspect_opened_session(&child).await;
        let (session, messages) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let _ = child.shutdown().await;
                return Err(error);
            }
        };
        if let Some(stored) = &stored_session {
            if stored != &session {
                let _ = child.shutdown().await;
                return Err(ChatRuntimeHostError::InvalidSessionState(
                    "the resumed Pi session did not match its stored id and path".into(),
                ));
            }
        } else {
            let inbox = Arc::clone(&self.inner.inbox);
            let owned_chat_id = chat_id.to_owned();
            let session_id = session.id.clone();
            let session_path = session.path.clone();
            let binding = tokio::task::spawn_blocking(move || {
                inbox.bind_chat_session(&owned_chat_id, &session_id, &session_path)
            })
            .await
            .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))?;
            if let Err(error) = binding {
                let _ = child.shutdown().await;
                return Err(error.into());
            }
        }

        let inbox = Arc::clone(&self.inner.inbox);
        let owned_chat_id = chat_id.to_owned();
        let initial_prompt =
            tokio::task::spawn_blocking(move || inbox.first_user_message(&owned_chat_id))
                .await
                .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))??;
        let session_is_empty = messages.is_empty();
        let should_start = stored_session.is_none() && session_is_empty;
        if !session_is_empty
            && first_user_text(&messages).as_deref() != Some(initial_prompt.as_str())
        {
            let _ = child.shutdown().await;
            return Err(ChatRuntimeHostError::InvalidSessionState(
                "the exact Pi session did not contain the chat's first message".into(),
            ));
        }

        let mut items = conversation_items(&messages);
        if session_is_empty {
            items.push(ConversationItem::Message {
                id: "message-0".into(),
                role: ConversationRole::User,
                text: initial_prompt.clone(),
            });
        }
        let snapshot = ConversationSnapshot {
            failure: None,
            items,
            phase: if should_start {
                ConversationPhase::Running
            } else if stored_session.is_some() {
                ConversationPhase::Stopped
            } else {
                ConversationPhase::Idle
            },
        };
        let next_message_index = messages
            .iter()
            .filter(|message| {
                matches!(
                    message.get("role").and_then(Value::as_str),
                    Some("user" | "assistant")
                )
            })
            .count()
            + usize::from(session_is_empty);
        let mut projection =
            ConversationProjection::new(snapshot.clone(), next_message_index, true);
        if should_start {
            projection
                .pending_user_items
                .push_back(("message-0".into(), initial_prompt.clone()));
        }
        *slot
            .projection
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)? = projection;
        if !should_start && !keep_stopped_runtime {
            child.shutdown().await?;
            return Ok(snapshot);
        }

        let stop_events = CancellationToken::new();
        let rpc_events = child.subscribe();
        *slot.active.lock().map_err(|_| ChatRuntimeHostError::Lock)? = Some(ActiveChat {
            child: Arc::clone(&child),
            command_generation: 0,
            stop_events: stop_events.clone(),
        });
        self.forward_events(
            chat_id.to_owned(),
            Arc::clone(&slot),
            Arc::clone(&child),
            rpc_events,
            stop_events,
        );
        if should_start
            && let Err(error) = child
                .request(
                    serde_json::json!({
                        "type": "prompt",
                        "message": initial_prompt,
                        "streamingBehavior": "steer"
                    }),
                    CancellationToken::new(),
                )
                .await
        {
            if let Some(active) = slot
                .active
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?
                .take()
            {
                active.stop_events.cancel();
            }
            let _ = child.shutdown().await;
            let mut failed = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            failed.snapshot.failure = Some(error.to_string());
            failed.snapshot.phase = ConversationPhase::Failed;
            return Err(error.into());
        }
        slot.projection
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)
            .map(|projection| projection.snapshot.clone())
    }

    pub async fn stop_runtime(&self, chat_id: &str) -> Result<(), ChatRuntimeHostError> {
        self.stop_slot(chat_id).await
    }

    async fn stop_slot(&self, chat_id: &str) -> Result<(), ChatRuntimeHostError> {
        let slot = self.slot(chat_id)?;
        let _operation = slot.operation.lock().await;
        let active = slot
            .active
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .take();
        if let Some(active) = active {
            active.stop_events.cancel();
            active.child.shutdown().await?;
        }
        let mut projection = slot
            .projection
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?;
        let changed = projection.snapshot.phase != ConversationPhase::Stopped;
        projection.snapshot.failure = None;
        projection.snapshot.phase = ConversationPhase::Stopped;
        drop(projection);
        if changed {
            self.emit(chat_id, ConversationEvent::TurnStopped);
        }
        Ok(())
    }

    pub async fn shutdown_all(&self) {
        let chat_ids = self
            .inner
            .slots
            .lock()
            .map(|slots| slots.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for chat_id in chat_ids {
            if let Err(error) = self.stop_runtime(&chat_id).await {
                tracing::warn!(%error, %chat_id, "could not stop chat runtime");
            }
        }
    }

    fn emit(&self, chat_id: &str, event: ConversationEvent) {
        let _ = self.inner.events.send(ChatRuntimeChangedEvent {
            chat_id: chat_id.to_owned(),
            event,
        });
    }

    fn forward_events(
        &self,
        chat_id: String,
        slot: Arc<ChatSlot>,
        child: Arc<PiRpcChild>,
        mut events: crate::pi_rpc::PiRpcEvents,
        stop_events: CancellationToken,
    ) {
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                let received = tokio::select! {
                    biased;
                    _ = stop_events.cancelled() => return,
                    event = events.recv() => event,
                };
                let Some(inner) = inner.upgrade() else {
                    return;
                };
                match received {
                    Ok(event) if is_terminal_pi_event(&event.payload) => {
                        let command_generation = {
                            let Ok(active) = slot.active.lock() else {
                                return;
                            };
                            let Some(active) = active
                                .as_ref()
                                .filter(|active| Arc::ptr_eq(&active.child, &child))
                            else {
                                return;
                            };
                            active.command_generation
                        };
                        let _operation = slot.operation.lock().await;
                        let projected = {
                            let Ok(mut active) = slot.active.lock() else {
                                return;
                            };
                            if !active
                                .as_ref()
                                .is_some_and(|active| Arc::ptr_eq(&active.child, &child))
                            {
                                return;
                            }
                            if active.as_ref().is_some_and(|active| {
                                active.command_generation != command_generation
                            }) {
                                continue;
                            }
                            let Ok(mut projection) = slot.projection.lock() else {
                                return;
                            };
                            let projected = project_pi_event(&mut projection, &event.payload);
                            let active = active.take();
                            (projected, active)
                        };
                        if let Some(active) = projected.1 {
                            active.stop_events.cancel();
                            if let Err(error) = active.child.shutdown().await {
                                tracing::warn!(%error, %chat_id, "could not retire completed chat runtime");
                            }
                        }
                        for event in projected.0 {
                            let _ = inner.events.send(ChatRuntimeChangedEvent {
                                chat_id: chat_id.clone(),
                                event,
                            });
                        }
                        return;
                    }
                    Ok(event) => {
                        let projected = {
                            let Ok(active) = slot.active.lock() else {
                                return;
                            };
                            if !active
                                .as_ref()
                                .is_some_and(|active| Arc::ptr_eq(&active.child, &child))
                            {
                                return;
                            }
                            let Ok(mut projection) = slot.projection.lock() else {
                                return;
                            };
                            project_pi_event(&mut projection, &event.payload)
                        };
                        for event in projected {
                            let _ = inner.events.send(ChatRuntimeChangedEvent {
                                chat_id: chat_id.clone(),
                                event,
                            });
                        }
                    }
                    Err(error) => {
                        if stop_events.is_cancelled() {
                            return;
                        }
                        let _operation = slot.operation.lock().await;
                        if stop_events.is_cancelled() {
                            return;
                        }
                        {
                            let Ok(mut active) = slot.active.lock() else {
                                return;
                            };
                            if !active
                                .as_ref()
                                .is_some_and(|active| Arc::ptr_eq(&active.child, &child))
                            {
                                return;
                            }
                            *active = None;
                        }
                        let message = error.to_string();
                        if let Ok(mut projection) = slot.projection.lock() {
                            projection.snapshot.failure = Some(message.clone());
                            projection.snapshot.phase = ConversationPhase::Failed;
                        }
                        if let Err(shutdown_error) = child.shutdown().await {
                            tracing::debug!(
                                %shutdown_error,
                                %chat_id,
                                "failed Pi runtime was already unavailable during retirement"
                            );
                        }
                        let _ = inner.events.send(ChatRuntimeChangedEvent {
                            chat_id: chat_id.clone(),
                            event: ConversationEvent::TurnFailed { message },
                        });
                        return;
                    }
                }
            }
        });
    }

    fn slot(&self, chat_id: &str) -> Result<Arc<ChatSlot>, ChatRuntimeHostError> {
        let mut slots = self
            .inner
            .slots
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?;
        Ok(Arc::clone(
            slots
                .entry(chat_id.to_owned())
                .or_insert_with(|| Arc::new(ChatSlot::new())),
        ))
    }

    fn process_spec(&self, worktree: &Path, session_path: Option<&Path>) -> PiRpcProcessSpec {
        let mut arguments = vec![
            self.inner.paths.launcher.as_os_str().to_owned(),
            flag("--cwd"),
            worktree.as_os_str().to_owned(),
            flag("--agent-dir"),
            self.inner.paths.agent_directory.as_os_str().to_owned(),
            flag("--session-dir"),
            self.inner.paths.session_directory.as_os_str().to_owned(),
            flag("--credential-lock-dir"),
            self.inner
                .paths
                .credential_lock_directory
                .as_os_str()
                .to_owned(),
            flag("--model-provider"),
            flag(MODEL_PROVIDER),
            flag("--model-id"),
            flag(MODEL_ID),
            flag("--thinking-level"),
            flag(THINKING_LEVEL),
        ];
        for skill_directory in [
            self.inner.paths.app_skill_directory.clone(),
            worktree.join(".pi/skills"),
        ] {
            if skill_directory.is_dir() {
                arguments.push(flag("--skill"));
                arguments.push(skill_directory.into_os_string());
            }
        }
        if let Some(session_path) = session_path {
            arguments.push(flag("--session-path"));
            arguments.push(session_path.as_os_str().to_owned());
        }
        let mut environment = BTreeMap::new();
        environment.insert(flag("HOME"), self.inner.paths.home.as_os_str().to_owned());
        environment.insert(flag("PATH"), self.inner.paths.executable_path.clone());
        environment.insert(
            flag("GIT_EXEC_PATH"),
            self.inner.paths.git_exec_path.as_os_str().to_owned(),
        );
        environment.insert(
            flag("GIT_TEMPLATE_DIR"),
            self.inner
                .paths
                .git_template_directory
                .as_os_str()
                .to_owned(),
        );
        environment.insert(flag("LC_ALL"), flag("C"));
        PiRpcProcessSpec {
            executable: self.inner.paths.node.clone(),
            arguments,
            working_directory: worktree.to_path_buf(),
            environment,
        }
    }

    async fn inspect_opened_session(
        &self,
        child: &PiRpcChild,
    ) -> Result<(ChatSessionReference, Vec<Value>), ChatRuntimeHostError> {
        let state = child
            .request(
                serde_json::json!({ "type": "get_state" }),
                CancellationToken::new(),
            )
            .await?
            .data
            .ok_or_else(|| {
                ChatRuntimeHostError::InvalidSessionState("get_state omitted data".into())
            })?;
        let session_id = state
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                ChatRuntimeHostError::InvalidSessionState("get_state omitted a session id".into())
            })?;
        let session_file = state
            .get("sessionFile")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|path| self.is_owned_session_path(path))
            .ok_or_else(|| {
                ChatRuntimeHostError::InvalidSessionState(
                    "get_state omitted an application-owned session path".into(),
                )
            })?;
        let messages = child
            .request(
                serde_json::json!({ "type": "get_messages" }),
                CancellationToken::new(),
            )
            .await?
            .data
            .and_then(|data| data.get("messages").and_then(Value::as_array).cloned())
            .ok_or_else(|| {
                ChatRuntimeHostError::InvalidSessionState("get_messages omitted messages".into())
            })?;
        Ok((
            ChatSessionReference {
                id: session_id.to_owned(),
                path: session_file,
            },
            messages,
        ))
    }

    fn is_owned_session_path(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.inner.paths.session_directory) else {
            return false;
        };
        path.is_absolute()
            && !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
    }
}

impl Drop for HostInner {
    fn drop(&mut self) {
        if let Ok(slots) = self.slots.lock() {
            for slot in slots.values() {
                if let Ok(active) = slot.active.lock()
                    && let Some(active) = active.as_ref()
                {
                    active.stop_events.cancel();
                }
            }
        }
    }
}

fn flag(value: &str) -> OsString {
    OsStr::new(value).to_owned()
}

fn is_terminal_pi_event(event: &Value) -> bool {
    event.get("type").and_then(Value::as_str) == Some("agent_end")
        && event.get("willRetry").and_then(Value::as_bool) != Some(true)
}

fn first_user_text(messages: &[Value]) -> Option<String> {
    messages.iter().find_map(|message| {
        (message.get("role").and_then(Value::as_str) == Some("user"))
            .then(|| message_content_text(message.get("content")))
            .flatten()
    })
}

fn message_content_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

fn conversation_items(messages: &[Value]) -> Vec<ConversationItem> {
    let mut items = Vec::new();
    let mut message_index = 0;
    for message in messages {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                if let Some(text) = message_content_text(message.get("content")) {
                    items.push(ConversationItem::Message {
                        id: format!("message-{message_index}"),
                        role: ConversationRole::User,
                        text,
                    });
                }
                message_index += 1;
            }
            Some("assistant") => {
                items.extend(assistant_items(message, message_index, true));
                message_index += 1;
            }
            Some("toolResult") => reconcile_tool_result(&mut items, message),
            _ => {}
        }
    }
    items
}

fn project_pi_event(
    projection: &mut ConversationProjection,
    event: &Value,
) -> Vec<ConversationEvent> {
    let mut emitted = Vec::new();
    match event.get("type").and_then(Value::as_str) {
        Some("agent_start") => {
            projection.snapshot.failure = None;
            projection.snapshot.phase = ConversationPhase::Running;
            emitted.push(ConversationEvent::TurnStarted);
        }
        Some("message_start") => {
            let Some(message) = event.get("message") else {
                return emitted;
            };
            match message.get("role").and_then(Value::as_str) {
                Some("user") => {
                    let Some(text) = message_content_text(message.get("content")) else {
                        return emitted;
                    };
                    if projection
                        .pending_user_items
                        .front()
                        .is_some_and(|(_, pending_text)| pending_text == &text)
                    {
                        projection.pending_user_items.pop_front();
                        return emitted;
                    }
                    projection.pending_user_items.clear();
                    let item = ConversationItem::Message {
                        id: format!("message-{}", projection.next_message_index),
                        role: ConversationRole::User,
                        text,
                    };
                    projection.next_message_index += 1;
                    add_item(projection, item, &mut emitted);
                }
                Some("assistant") => {
                    let message_index = projection.next_message_index;
                    projection.next_message_index += 1;
                    projection.active_assistant_index = Some(message_index);
                    projection.tool_content_ids.clear();
                    for item in assistant_items(message, message_index, false) {
                        add_item(projection, item, &mut emitted);
                    }
                }
                _ => {}
            }
        }
        Some("message_update") => {
            let Some(message_index) = projection.active_assistant_index else {
                return emitted;
            };
            let Some(update) = event.get("assistantMessageEvent") else {
                return emitted;
            };
            let content_index = update
                .get("contentIndex")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok());
            match update.get("type").and_then(Value::as_str) {
                Some("text_start") => {
                    if let Some(content_index) = content_index {
                        add_item(
                            projection,
                            ConversationItem::Message {
                                id: text_item_id(message_index, content_index),
                                role: ConversationRole::Assistant,
                                text: String::new(),
                            },
                            &mut emitted,
                        );
                    }
                }
                Some("text_delta") => {
                    if let (Some(content_index), Some(delta)) =
                        (content_index, update.get("delta").and_then(Value::as_str))
                    {
                        let item_id = text_item_id(message_index, content_index);
                        append_item_text(projection, &item_id, delta, false, &mut emitted);
                    }
                }
                Some("thinking_start") => {
                    if let Some(content_index) = content_index {
                        add_item(
                            projection,
                            ConversationItem::Reasoning {
                                id: reasoning_item_id(message_index, content_index),
                                text: String::new(),
                            },
                            &mut emitted,
                        );
                    }
                }
                Some("thinking_delta") => {
                    if let (Some(content_index), Some(delta)) =
                        (content_index, update.get("delta").and_then(Value::as_str))
                    {
                        let item_id = reasoning_item_id(message_index, content_index);
                        append_item_text(projection, &item_id, delta, true, &mut emitted);
                    }
                }
                Some("toolcall_start") => {
                    if let (Some(content_index), Some(tool_call_id), Some(tool_name)) = (
                        content_index,
                        update.get("id").and_then(Value::as_str),
                        update.get("toolName").and_then(Value::as_str),
                    ) {
                        let item_id = tool_item_id(tool_call_id);
                        projection
                            .tool_content_ids
                            .insert(content_index, item_id.clone());
                        add_item(
                            projection,
                            ConversationItem::Tool {
                                detail: String::new(),
                                id: item_id,
                                name: tool_name.to_owned(),
                                status: ConversationToolStatus::Running,
                            },
                            &mut emitted,
                        );
                    }
                }
                Some("toolcall_delta") => {
                    if let (Some(content_index), Some(delta)) =
                        (content_index, update.get("delta").and_then(Value::as_str))
                        && let Some(item_id) =
                            projection.tool_content_ids.get(&content_index).cloned()
                    {
                        update_tool(
                            projection,
                            &item_id,
                            None,
                            Some(delta),
                            ConversationToolStatus::Running,
                            &mut emitted,
                        );
                    }
                }
                Some("toolcall_end") => {
                    if let Some(tool_call) = update.get("toolCall")
                        && let (Some(tool_call_id), Some(tool_name)) = (
                            tool_call.get("id").and_then(Value::as_str),
                            tool_call.get("name").and_then(Value::as_str),
                        )
                    {
                        let item_id = tool_item_id(tool_call_id);
                        if let Some(content_index) = content_index {
                            projection
                                .tool_content_ids
                                .insert(content_index, item_id.clone());
                        }
                        update_tool(
                            projection,
                            &item_id,
                            Some(tool_name),
                            tool_call.get("arguments").map(display_json).as_deref(),
                            ConversationToolStatus::Running,
                            &mut emitted,
                        );
                    }
                }
                _ => {}
            }
            if matches!(
                update.get("type").and_then(Value::as_str),
                Some("done" | "error")
            ) && let Some(usage) = event.get("usage")
            {
                reconcile_usage(projection, message_index, usage, &mut emitted);
            }
        }
        Some("tool_execution_start") => {
            if let (Some(tool_call_id), Some(tool_name)) = (
                event.get("toolCallId").and_then(Value::as_str),
                event.get("toolName").and_then(Value::as_str),
            ) {
                update_tool(
                    projection,
                    &tool_item_id(tool_call_id),
                    Some(tool_name),
                    event.get("args").map(display_json).as_deref(),
                    ConversationToolStatus::Running,
                    &mut emitted,
                );
            }
        }
        Some("tool_execution_update") => {
            if let (Some(tool_call_id), Some(tool_name)) = (
                event.get("toolCallId").and_then(Value::as_str),
                event.get("toolName").and_then(Value::as_str),
            ) {
                let detail = event
                    .get("partialResult")
                    .map(result_detail)
                    .unwrap_or_default();
                update_tool(
                    projection,
                    &tool_item_id(tool_call_id),
                    Some(tool_name),
                    Some(&detail),
                    ConversationToolStatus::Running,
                    &mut emitted,
                );
            }
        }
        Some("tool_execution_end") => {
            if let (Some(tool_call_id), Some(tool_name)) = (
                event.get("toolCallId").and_then(Value::as_str),
                event.get("toolName").and_then(Value::as_str),
            ) {
                let detail = event.get("result").map(result_detail).unwrap_or_default();
                let status = if event.get("isError").and_then(Value::as_bool) == Some(true) {
                    ConversationToolStatus::Failed
                } else {
                    ConversationToolStatus::Succeeded
                };
                update_tool(
                    projection,
                    &tool_item_id(tool_call_id),
                    Some(tool_name),
                    Some(&detail),
                    status,
                    &mut emitted,
                );
            }
        }
        Some("message_end") => {
            if let Some(message) = event.get("message")
                && message.get("role").and_then(Value::as_str) == Some("assistant")
                && let Some(message_index) = projection.active_assistant_index
            {
                for item in assistant_items(message, message_index, true) {
                    reconcile_authoritative_item(projection, item, &mut emitted);
                }
            }
        }
        Some("agent_end") if event.get("willRetry").and_then(Value::as_bool) == Some(true) => {}
        Some("agent_end") => {
            let failure = event
                .get("messages")
                .and_then(Value::as_array)
                .and_then(|messages| {
                    messages.iter().rev().find(|message| {
                        message.get("role").and_then(Value::as_str) == Some("assistant")
                    })
                });
            match failure.and_then(|message| message.get("stopReason").and_then(Value::as_str)) {
                Some("aborted") => {
                    projection.snapshot.failure = None;
                    projection.snapshot.phase = ConversationPhase::Stopped;
                    emitted.push(ConversationEvent::TurnStopped);
                }
                Some("error") => {
                    let message = failure
                        .and_then(|message| message.get("errorMessage"))
                        .and_then(Value::as_str)
                        .unwrap_or("The agent turn failed.")
                        .to_owned();
                    projection.snapshot.failure = Some(message.clone());
                    projection.snapshot.phase = ConversationPhase::Failed;
                    emitted.push(ConversationEvent::TurnFailed { message });
                }
                _ => {
                    projection.snapshot.failure = None;
                    projection.snapshot.phase = ConversationPhase::Idle;
                    emitted.push(ConversationEvent::TurnCompleted);
                }
            }
            projection.active_assistant_index = None;
            projection.tool_content_ids.clear();
        }
        _ => {}
    }
    emitted
}

fn add_item(
    projection: &mut ConversationProjection,
    item: ConversationItem,
    emitted: &mut Vec<ConversationEvent>,
) {
    if projection
        .snapshot
        .items
        .iter()
        .any(|existing| existing.id() == item.id())
    {
        return;
    }
    projection.snapshot.items.push(item.clone());
    emitted.push(ConversationEvent::ItemAdded { item });
}

fn append_item_text(
    projection: &mut ConversationProjection,
    item_id: &str,
    delta: &str,
    reasoning: bool,
    emitted: &mut Vec<ConversationEvent>,
) {
    let Some(item) = projection
        .snapshot
        .items
        .iter_mut()
        .find(|item| item.id() == item_id)
    else {
        return;
    };
    match item {
        ConversationItem::Message { text, .. } if !reasoning => text.push_str(delta),
        ConversationItem::Reasoning { text, .. } if reasoning => text.push_str(delta),
        _ => return,
    }
    emitted.push(if reasoning {
        ConversationEvent::ReasoningDelta {
            delta: delta.to_owned(),
            item_id: item_id.to_owned(),
        }
    } else {
        ConversationEvent::TextDelta {
            delta: delta.to_owned(),
            item_id: item_id.to_owned(),
        }
    });
}

fn update_tool(
    projection: &mut ConversationProjection,
    item_id: &str,
    name: Option<&str>,
    detail: Option<&str>,
    status: ConversationToolStatus,
    emitted: &mut Vec<ConversationEvent>,
) {
    let existing = projection
        .snapshot
        .items
        .iter_mut()
        .find(|item| item.id() == item_id);
    if let Some(ConversationItem::Tool {
        detail: current_detail,
        name: current_name,
        status: current_status,
        ..
    }) = existing
    {
        if let Some(name) = name {
            *current_name = name.to_owned();
        }
        if let Some(detail) = detail {
            *current_detail = detail.to_owned();
        }
        *current_status = status;
        emitted.push(ConversationEvent::ToolUpdate {
            detail: current_detail.clone(),
            item_id: item_id.to_owned(),
            status,
        });
        return;
    }
    let item = ConversationItem::Tool {
        detail: detail.unwrap_or_default().to_owned(),
        id: item_id.to_owned(),
        name: name.unwrap_or("tool").to_owned(),
        status,
    };
    add_item(projection, item, emitted);
}

fn reconcile_usage(
    projection: &mut ConversationProjection,
    message_index: usize,
    usage: &Value,
    emitted: &mut Vec<ConversationEvent>,
) {
    let item_id = usage_item_id(message_index);
    let input_tokens = usage.get("input").and_then(Value::as_u64).unwrap_or(0);
    let output_tokens = usage.get("output").and_then(Value::as_u64).unwrap_or(0);
    let cache_read_tokens = usage.get("cacheRead").and_then(Value::as_u64);
    if let Some(ConversationItem::Usage {
        cache_read_tokens: current_cache,
        input_tokens: current_input,
        output_tokens: current_output,
        ..
    }) = projection
        .snapshot
        .items
        .iter_mut()
        .find(|item| item.id() == item_id)
    {
        *current_cache = cache_read_tokens;
        *current_input = input_tokens;
        *current_output = output_tokens;
        emitted.push(ConversationEvent::UsageUpdate {
            cache_read_tokens,
            input_tokens,
            item_id,
            output_tokens,
        });
    } else {
        add_item(
            projection,
            ConversationItem::Usage {
                cache_read_tokens,
                id: item_id,
                input_tokens,
                output_tokens,
            },
            emitted,
        );
    }
}

fn reconcile_authoritative_item(
    projection: &mut ConversationProjection,
    item: ConversationItem,
    emitted: &mut Vec<ConversationEvent>,
) {
    let item_id = item.id().to_owned();
    let Some(existing) = projection
        .snapshot
        .items
        .iter()
        .find(|existing| existing.id() == item_id)
        .cloned()
    else {
        add_item(projection, item, emitted);
        return;
    };
    match (existing, item) {
        (
            ConversationItem::Message { text: current, .. },
            ConversationItem::Message {
                text: final_text, ..
            },
        ) if final_text.starts_with(&current) => {
            append_item_text(
                projection,
                &item_id,
                &final_text[current.len()..],
                false,
                emitted,
            );
        }
        (
            ConversationItem::Reasoning { text: current, .. },
            ConversationItem::Reasoning {
                text: final_text, ..
            },
        ) if final_text.starts_with(&current) => {
            append_item_text(
                projection,
                &item_id,
                &final_text[current.len()..],
                true,
                emitted,
            );
        }
        (
            ConversationItem::Usage { .. },
            ConversationItem::Usage {
                cache_read_tokens,
                input_tokens,
                output_tokens,
                ..
            },
        ) => reconcile_usage(
            projection,
            item_id
                .strip_prefix("message-")
                .and_then(|rest| rest.strip_suffix("-usage"))
                .and_then(|index| index.parse().ok())
                .unwrap_or_default(),
            &serde_json::json!({
                "input": input_tokens,
                "output": output_tokens,
                "cacheRead": cache_read_tokens
            }),
            emitted,
        ),
        _ => {}
    }
}

fn assistant_items(
    message: &Value,
    message_index: usize,
    include_usage: bool,
) -> Vec<ConversationItem> {
    let mut items = Vec::new();
    if let Some(content) = message.get("content").and_then(Value::as_array) {
        for (content_index, part) in content.iter().enumerate() {
            match part.get("type").and_then(Value::as_str) {
                Some("text") => items.push(ConversationItem::Message {
                    id: text_item_id(message_index, content_index),
                    role: ConversationRole::Assistant,
                    text: part
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }),
                Some("thinking") => items.push(ConversationItem::Reasoning {
                    id: reasoning_item_id(message_index, content_index),
                    text: part
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }),
                Some("toolCall") => {
                    if let (Some(id), Some(name)) = (
                        part.get("id").and_then(Value::as_str),
                        part.get("name").and_then(Value::as_str),
                    ) {
                        items.push(ConversationItem::Tool {
                            detail: part.get("arguments").map(display_json).unwrap_or_default(),
                            id: tool_item_id(id),
                            name: name.to_owned(),
                            status: ConversationToolStatus::Running,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    if include_usage && let Some(usage) = message.get("usage") {
        items.push(ConversationItem::Usage {
            cache_read_tokens: usage.get("cacheRead").and_then(Value::as_u64),
            id: usage_item_id(message_index),
            input_tokens: usage.get("input").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: usage.get("output").and_then(Value::as_u64).unwrap_or(0),
        });
    }
    items
}

fn reconcile_tool_result(items: &mut [ConversationItem], message: &Value) {
    let Some(tool_call_id) = message.get("toolCallId").and_then(Value::as_str) else {
        return;
    };
    let item_id = tool_item_id(tool_call_id);
    if let Some(ConversationItem::Tool { detail, status, .. }) =
        items.iter_mut().find(|item| item.id() == item_id)
    {
        *detail = result_detail(message);
        *status = if message.get("isError").and_then(Value::as_bool) == Some(true) {
            ConversationToolStatus::Failed
        } else {
            ConversationToolStatus::Succeeded
        };
    }
}

fn result_detail(result: &Value) -> String {
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        display_json(result)
    } else {
        text
    }
}

fn display_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "Unable to display tool data".into())
}

fn text_item_id(message_index: usize, content_index: usize) -> String {
    format!("message-{message_index}-text-{content_index}")
}

fn reasoning_item_id(message_index: usize, content_index: usize) -> String {
    format!("message-{message_index}-reasoning-{content_index}")
}

fn usage_item_id(message_index: usize) -> String {
    format!("message-{message_index}-usage")
}

fn tool_item_id(tool_call_id: &str) -> String {
    format!("tool-{tool_call_id}")
}

impl ConversationItem {
    fn id(&self) -> &str {
        match self {
            Self::Message { id, .. }
            | Self::Reasoning { id, .. }
            | Self::Tool { id, .. }
            | Self::Usage { id, .. } => id,
        }
    }
}

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, broadcast};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::{
    agent_environment::{
        AgentEnvironment, AgentEnvironmentError, AgentLaunchResources, AgentResourceId,
        AgentResourcePreferenceChange, AgentResourcePreferenceScope, AgentResourceRefreshStatus,
    },
    chat_workspaces::{ChatAgentLaunchContext, ChatWorkspaceError, ChatWorkspaces},
    pi_rpc::{PiRpcChild, PiRpcError, PiRpcExtensionUiResponse, PiRpcPolicy, PiRpcProcessSpec},
    project_inbox::{ChatSessionReference, ChatSetupPhase, ProjectInbox, ProjectInboxError},
    prompt_attachments::{
        PromptAttachment, PromptAttachmentError, image_payloads, prompt_text,
        validate as validate_attachments,
    },
    runtime_preferences::{
        ModelRoute as PersistedModelRoute, RuntimePreferences, RuntimePreferencesError,
    },
};

const THINKING_LEVEL: &str = "xhigh";
const RESTORED_INTERRUPTED_TURN_MESSAGE: &str =
    "The agent turn was interrupted before Più reopened this chat.";
const RUNTIME_INTERRUPTED_TURN_MESSAGE: &str =
    "The agent runtime stopped before the turn finished. Send another message to continue.";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationPhase {
    Idle,
    Running,
    Stopped,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ModelRouteId {
    pub provider: String,
    pub model_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ModelRouteSummary {
    pub id: ModelRouteId,
    pub name: String,
    pub accepts_images: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ReasoningEffort {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    #[ts(rename = "xhigh")]
    ExtraHigh,
    #[serde(rename = "max")]
    #[ts(rename = "max")]
    Maximum,
}

impl ReasoningEffort {
    pub(crate) fn from_pi(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::ExtraHigh),
            "max" => Some(Self::Maximum),
            _ => None,
        }
    }

    pub(crate) fn as_pi(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::ExtraHigh => "xhigh",
            Self::Maximum => "max",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ModelControlsSnapshot {
    pub routes: Vec<ModelRouteSummary>,
    pub selected_route: ModelRouteId,
    pub efforts: Vec<ReasoningEffort>,
    pub selected_effort: ReasoningEffort,
    pub applies_after_current_step: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ConversationSnapshot {
    pub failure: Option<String>,
    pub input_request: Option<ConversationInputRequest>,
    pub items: Vec<ConversationItem>,
    pub phase: ConversationPhase,
    #[ts(type = "number")]
    pub revision: u64,
}

impl ConversationSnapshot {
    fn stopped() -> Self {
        Self {
            failure: None,
            input_request: None,
            items: Vec::new(),
            phase: ConversationPhase::Stopped,
            revision: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationInputKind {
    Select,
    Confirm,
    Input,
    Editor,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ConversationInputRequest {
    pub id: String,
    pub kind: ConversationInputKind,
    pub title: String,
    pub message: Option<String>,
    pub options: Vec<String>,
    pub placeholder: Option<String>,
    pub prefill: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationInputAnswer {
    Value { value: String },
    Confirmed { confirmed: bool },
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationItem {
    Message {
        id: String,
        queued: bool,
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
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationEvent {
    ItemAdded {
        #[serde(rename = "beforeItemId")]
        before_item_id: Option<String>,
        item: ConversationItem,
    },
    ItemRemoved {
        #[serde(rename = "itemId")]
        item_id: String,
    },
    MessageQueueChanged {
        #[serde(rename = "itemId")]
        item_id: String,
        queued: bool,
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
    InputRequested {
        request: ConversationInputRequest,
    },
    InputResolved {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    TurnCompleted,
    TurnStopped,
    TurnInterrupted {
        message: String,
    },
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
    #[ts(type = "number")]
    pub revision: u64,
}

#[derive(Debug, Error)]
pub enum ChatRuntimeHostError {
    #[error("a chat message cannot be empty")]
    EmptyMessage,
    #[error("chat {chat_id} has no active Pi runtime")]
    NotActive { chat_id: String },
    #[error("chat {chat_id} is not waiting for input {request_id}")]
    InputNotPending { chat_id: String, request_id: String },
    #[error("the answer does not match the pending extension input")]
    InvalidInputAnswer,
    #[error("model route {provider}/{model_id} is unavailable")]
    ModelUnavailable { provider: String, model_id: String },
    #[error("reasoning effort {effort:?} is unavailable for the selected model")]
    EffortUnavailable { effort: ReasoningEffort },
    #[error("Pi did not retain the requested model controls")]
    InferenceChangeRejected,
    #[error("Pi rejected the inference change and could not restore the previous controls")]
    InferenceRollbackFailed,
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
    #[error(transparent)]
    Attachment(#[from] PromptAttachmentError),
    #[error(transparent)]
    Preferences(#[from] RuntimePreferencesError),
    #[error(transparent)]
    Environment(#[from] AgentEnvironmentError),
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
    pending_resource_refresh: Mutex<bool>,
    project_id: Mutex<Option<i64>>,
    projection: Mutex<ConversationProjection>,
}

impl ChatSlot {
    fn new() -> Self {
        Self {
            operation: AsyncMutex::new(()),
            active: Mutex::new(None),
            pending_resource_refresh: Mutex::new(false),
            project_id: Mutex::new(None),
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
    launch_resources: AgentLaunchResources,
    project_id: i64,
    send_only: bool,
    stop_events: CancellationToken,
    worktree_path: PathBuf,
}

#[derive(Clone)]
struct CachedProjectEnvironment {
    canonical_resources: AgentLaunchResources,
    model_controls: ModelControlsSnapshot,
    project_root: PathBuf,
}

struct ConversationProjection {
    snapshot: ConversationSnapshot,
    hydrated: bool,
    next_message_index: usize,
    active_assistant_index: Option<usize>,
    tool_content_ids: HashMap<usize, String>,
    native_steering_count: usize,
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
            native_steering_count: 0,
            pending_user_items: VecDeque::new(),
        }
    }

    fn stamp_events(&mut self, events: Vec<ConversationEvent>) -> Vec<(u64, ConversationEvent)> {
        events
            .into_iter()
            .map(|event| {
                self.snapshot.revision = self
                    .snapshot
                    .revision
                    .checked_add(1)
                    .expect("conversation event revision overflowed");
                (self.snapshot.revision, event)
            })
            .collect()
    }

    fn install_opened_session(
        &mut self,
        snapshot: ConversationSnapshot,
        next_message_index: usize,
        preserve_hydrated: bool,
    ) -> bool {
        if preserve_hydrated && self.hydrated {
            return false;
        }
        *self = Self::new(snapshot, next_message_index, true);
        true
    }
}

struct HostInner {
    inbox: Arc<ProjectInbox>,
    workspaces: Arc<ChatWorkspaces>,
    environment: Arc<AgentEnvironment>,
    preferences: Arc<RuntimePreferences>,
    paths: RuntimePaths,
    project_environments: Mutex<HashMap<i64, CachedProjectEnvironment>>,
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
        environment: Arc<AgentEnvironment>,
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
            environment,
            app_data_directory,
            resource_directory,
            &home,
        )
    }

    pub fn new(
        inbox: Arc<ProjectInbox>,
        workspaces: Arc<ChatWorkspaces>,
        environment: Arc<AgentEnvironment>,
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
                environment,
                preferences: Arc::new(RuntimePreferences::open(
                    &app_data_directory.join("piu.sqlite3"),
                )?),
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
                project_environments: Mutex::new(HashMap::new()),
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

    async fn chat_launch_context(
        &self,
        chat_id: &str,
    ) -> Result<ChatAgentLaunchContext, ChatRuntimeHostError> {
        let workspaces = Arc::clone(&self.inner.workspaces);
        let owned_chat_id = chat_id.to_owned();
        tokio::task::spawn_blocking(move || workspaces.agent_launch_context(&owned_chat_id))
            .await
            .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))?
            .map_err(Into::into)
    }

    async fn inspect_project_environment(
        &self,
        project_id: i64,
    ) -> Result<CachedProjectEnvironment, ChatRuntimeHostError> {
        let environment = Arc::clone(&self.inner.environment);
        let environment = environment.project_runtime_environment(project_id);
        let inbox = Arc::clone(&self.inner.inbox);
        let project = tokio::task::spawn_blocking(move || inbox.project_location(project_id));
        let (environment, project) = tokio::try_join!(
            async { environment.await.map_err(ChatRuntimeHostError::from) },
            async {
                project
                    .await
                    .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))?
                    .map_err(ChatRuntimeHostError::from)
            }
        )?;
        Ok(CachedProjectEnvironment {
            canonical_resources: environment.launch_resources,
            model_controls: environment.model_controls,
            project_root: project.canonical_path,
        })
    }

    fn cached_project_environment(
        &self,
        project_id: i64,
    ) -> Result<Option<CachedProjectEnvironment>, ChatRuntimeHostError> {
        self.inner
            .project_environments
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)
            .map(|projects| projects.get(&project_id).cloned())
    }

    fn cache_project_environment(
        &self,
        project_id: i64,
        environment: CachedProjectEnvironment,
    ) -> Result<(), ChatRuntimeHostError> {
        self.inner
            .project_environments
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .insert(project_id, environment);
        Ok(())
    }

    async fn ensure_project_environment(
        &self,
        project_id: i64,
        inspect_on_miss: bool,
    ) -> Result<CachedProjectEnvironment, ChatRuntimeHostError> {
        if let Some(environment) = self.cached_project_environment(project_id)? {
            return Ok(environment);
        }
        if !inspect_on_miss {
            return Err(ChatRuntimeHostError::InvalidSessionState(
                "the chat environment was not prepared before entering the send path".into(),
            ));
        }
        let environment = self.inspect_project_environment(project_id).await?;
        self.cache_project_environment(project_id, environment.clone())?;
        Ok(environment)
    }

    async fn prepare_launch(
        &self,
        chat_id: &str,
        inspect_on_miss: bool,
    ) -> Result<
        (
            ChatAgentLaunchContext,
            AgentLaunchResources,
            ModelControlsSnapshot,
        ),
        ChatRuntimeHostError,
    > {
        let context = self.chat_launch_context(chat_id).await?;
        let environment = self
            .ensure_project_environment(context.project_id, inspect_on_miss)
            .await?;
        let launch_resources = remap_launch_resources(
            &environment.canonical_resources,
            &environment.project_root,
            &context.worktree_path,
        )
        .await?;
        Ok((context, launch_resources, environment.model_controls))
    }

    pub async fn refresh_resources(
        &self,
        changed_project_id: i64,
        mut change: AgentResourcePreferenceChange,
    ) -> Result<AgentResourcePreferenceChange, ChatRuntimeHostError> {
        let slots = self
            .inner
            .slots
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .iter()
            .map(|(chat_id, slot)| (chat_id.clone(), Arc::clone(slot)))
            .collect::<Vec<_>>();
        let model_change = matches!(&change.resource, AgentResourceId::ModelRoute { .. });
        let mut candidates = Vec::new();
        for (chat_id, slot) in slots {
            let active_snapshot = {
                let active = slot.active.lock().map_err(|_| ChatRuntimeHostError::Lock)?;
                active.as_ref().map(|active| {
                    (
                        Arc::clone(&active.child),
                        active.project_id,
                        active.launch_resources.clone(),
                        active.worktree_path.clone(),
                    )
                })
            };
            let project_id = active_snapshot
                .as_ref()
                .map(|(_, project_id, _, _)| *project_id)
                .or(*slot
                    .project_id
                    .lock()
                    .map_err(|_| ChatRuntimeHostError::Lock)?);
            let Some(project_id) = project_id else {
                continue;
            };
            if change.scope == AgentResourcePreferenceScope::Project
                && project_id != changed_project_id
            {
                continue;
            }
            let Some((child, _, launch_resources, worktree_path)) = active_snapshot else {
                *slot
                    .pending_resource_refresh
                    .lock()
                    .map_err(|_| ChatRuntimeHostError::Lock)? = true;
                continue;
            };
            candidates.push((
                chat_id,
                slot,
                child,
                project_id,
                launch_resources,
                worktree_path,
            ));
        }

        let previous_environments = self
            .inner
            .project_environments
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .clone();
        let mut project_ids = candidates
            .iter()
            .map(|(_, _, _, project_id, _, _)| *project_id)
            .collect::<HashSet<_>>();
        project_ids.insert(changed_project_id);
        let environments = join_all(project_ids.into_iter().map(|project_id| async move {
            self.inspect_project_environment(project_id)
                .await
                .map(|environment| (project_id, environment))
        }))
        .await
        .into_iter()
        .collect::<Result<HashMap<_, _>, _>>()?;

        let mut planned = Vec::with_capacity(candidates.len());
        for (chat_id, slot, child, project_id, launched_resources, worktree_path) in candidates {
            let environment = environments.get(&project_id).ok_or_else(|| {
                ChatRuntimeHostError::InvalidSessionState(
                    "an affected project had no prepared environment".into(),
                )
            })?;
            let effective_resources = if model_change {
                launched_resources.clone()
            } else {
                remap_launch_resources(
                    &environment.canonical_resources,
                    &environment.project_root,
                    &worktree_path,
                )
                .await?
            };
            let reconcile_model = model_change
                || previous_environments
                    .get(&project_id)
                    .is_none_or(|previous| previous.model_controls != environment.model_controls);
            planned.push((
                chat_id,
                slot,
                child,
                launched_resources,
                effective_resources,
                reconcile_model.then(|| environment.model_controls.clone()),
            ));
        }

        {
            let mut cached = self
                .inner
                .project_environments
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            match change.scope {
                AgentResourcePreferenceScope::Global => cached.clear(),
                AgentResourcePreferenceScope::Project => {
                    cached.remove(&changed_project_id);
                }
            }
            cached.extend(environments);
        }

        let mut restart = Vec::new();
        let mut deferred_chat_count = 0_u32;
        let mut restart_failed_chat_count = 0_u32;
        for (
            chat_id,
            slot,
            planned_child,
            launched_resources,
            effective_resources,
            model_controls,
        ) in planned
        {
            let operation = slot.operation.lock().await;
            let still_planned_child = slot
                .active
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(&active.child, &planned_child));
            if !still_planned_child {
                continue;
            }
            let resources_changed = effective_resources != launched_resources;
            if !resources_changed && model_controls.is_none() {
                *slot
                    .pending_resource_refresh
                    .lock()
                    .map_err(|_| ChatRuntimeHostError::Lock)? = false;
                continue;
            }
            let running = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?
                .snapshot
                .phase
                == ConversationPhase::Running;
            if running {
                *slot
                    .pending_resource_refresh
                    .lock()
                    .map_err(|_| ChatRuntimeHostError::Lock)? = true;
                deferred_chat_count = deferred_chat_count.saturating_add(1);
                continue;
            }
            if let Some(model_controls) = model_controls {
                match self
                    .reconcile_model_controls_snapshot(&slot, &planned_child, model_controls)
                    .await
                {
                    Ok(_) => {
                        *slot
                            .pending_resource_refresh
                            .lock()
                            .map_err(|_| ChatRuntimeHostError::Lock)? = false;
                        if !resources_changed {
                            continue;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, %chat_id, "could not apply the enabled model fallback");
                    }
                }
            }
            *slot
                .pending_resource_refresh
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)? = false;
            let active = slot
                .active
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?
                .take();
            if let Some(active) = active {
                active.stop_events.cancel();
                if let Err(error) = active.child.shutdown().await {
                    tracing::warn!(%error, %chat_id, "could not retire chat while applying resources");
                    *slot
                        .pending_resource_refresh
                        .lock()
                        .map_err(|_| ChatRuntimeHostError::Lock)? = true;
                    restart_failed_chat_count = restart_failed_chat_count.saturating_add(1);
                    continue;
                }
                restart.push((chat_id, true));
            }
            drop(operation);
        }
        for (chat_id, reconcile_model) in restart {
            let reopened = self.open_for_send(&chat_id).await.map(|_| ());
            let reopened = match reopened {
                Ok(()) if reconcile_model => self.model_controls(&chat_id).await.map(|_| ()),
                other => other,
            };
            if let Err(error) = reopened {
                tracing::warn!(%error, %chat_id, "could not reopen chat after applying resources");
                if let Ok(slot) = self.slot(&chat_id)
                    && let Ok(mut pending) = slot.pending_resource_refresh.lock()
                {
                    *pending = true;
                }
                restart_failed_chat_count = restart_failed_chat_count.saturating_add(1);
            }
        }
        change.deferred_chat_count = deferred_chat_count;
        change.restart_failed_chat_count = restart_failed_chat_count;
        change.status = if restart_failed_chat_count > 0 {
            AgentResourceRefreshStatus::RestartFailed
        } else if deferred_chat_count > 0 {
            AgentResourceRefreshStatus::Deferred
        } else {
            AgentResourceRefreshStatus::Applied
        };
        Ok(change)
    }

    pub async fn model_controls(
        &self,
        chat_id: &str,
    ) -> Result<ModelControlsSnapshot, ChatRuntimeHostError> {
        self.open_slot(chat_id, true, true).await?;
        let slot = self.slot(chat_id)?;
        let _operation = slot.operation.lock().await;
        let (child, project_id) = active_model_context(&slot, chat_id)?;
        self.effective_model_controls_snapshot(&slot, &child, project_id)
            .await
    }

    pub async fn select_model_route(
        &self,
        chat_id: &str,
        route: ModelRouteId,
    ) -> Result<ModelControlsSnapshot, ChatRuntimeHostError> {
        self.open_slot(chat_id, true, true).await?;
        let slot = self.slot(chat_id)?;
        let operation = slot.operation.lock().await;
        let (child, project_id) = active_model_context(&slot, chat_id)?;
        let previous = self
            .effective_model_controls_snapshot(&slot, &child, project_id)
            .await?;
        if !previous
            .routes
            .iter()
            .any(|available| available.id == route)
        {
            return Err(ChatRuntimeHostError::ModelUnavailable {
                provider: route.provider,
                model_id: route.model_id,
            });
        }
        let persisted_route = PersistedModelRoute::new(&route.provider, &route.model_id)?;
        let remembered_effort =
            remembered_effort(Arc::clone(&self.inner.preferences), persisted_route.clone()).await?;
        let changed = async {
            child
                .request(
                    serde_json::json!({
                        "type": "set_model",
                        "provider": &route.provider,
                        "modelId": &route.model_id,
                    }),
                    CancellationToken::new(),
                )
                .await?;
            let mut applied = self
                .effective_model_controls_snapshot(&slot, &child, project_id)
                .await?;
            if applied.selected_route != route {
                return Err(ChatRuntimeHostError::InferenceChangeRejected);
            }
            if let Some(remembered_effort) = remembered_effort
                && applied.efforts.contains(&remembered_effort)
                && applied.selected_effort != remembered_effort
            {
                child
                    .request(
                        serde_json::json!({
                            "type": "set_thinking_level",
                            "level": remembered_effort.as_pi(),
                        }),
                        CancellationToken::new(),
                    )
                    .await?;
                applied = self
                    .effective_model_controls_snapshot(&slot, &child, project_id)
                    .await?;
                if applied.selected_effort != remembered_effort {
                    return Err(ChatRuntimeHostError::InferenceChangeRejected);
                }
            }
            persist_selected_route(
                Arc::clone(&self.inner.preferences),
                persisted_route,
                applied.selected_effort,
            )
            .await?;
            Ok::<_, ChatRuntimeHostError>(applied)
        }
        .await;
        match changed {
            Ok(applied) => {
                self.cache_applied_model_controls(project_id, &applied)?;
                Ok(applied)
            }
            Err(error) => {
                if rollback_model_controls(&slot, &child, &previous)
                    .await
                    .is_ok()
                {
                    return Err(error);
                }
                let retired = retire_child_for_recovery(&slot, &child, chat_id).await;
                drop(operation);
                if retired {
                    let _ = self.open_for_send(chat_id).await;
                }
                Err(ChatRuntimeHostError::InferenceRollbackFailed)
            }
        }
    }

    pub async fn select_reasoning_effort(
        &self,
        chat_id: &str,
        effort: ReasoningEffort,
    ) -> Result<ModelControlsSnapshot, ChatRuntimeHostError> {
        self.open_slot(chat_id, true, true).await?;
        let slot = self.slot(chat_id)?;
        let operation = slot.operation.lock().await;
        let (child, project_id) = active_model_context(&slot, chat_id)?;
        let previous = self
            .effective_model_controls_snapshot(&slot, &child, project_id)
            .await?;
        if !previous.efforts.contains(&effort) {
            return Err(ChatRuntimeHostError::EffortUnavailable { effort });
        }
        let changed = async {
            child
                .request(
                    serde_json::json!({
                        "type": "set_thinking_level",
                        "level": effort.as_pi(),
                    }),
                    CancellationToken::new(),
                )
                .await?;
            let snapshot = self
                .effective_model_controls_snapshot(&slot, &child, project_id)
                .await?;
            if snapshot.selected_effort != effort {
                return Err(ChatRuntimeHostError::InferenceChangeRejected);
            }
            let route = PersistedModelRoute::new(
                &snapshot.selected_route.provider,
                &snapshot.selected_route.model_id,
            )?;
            persist_effort(
                Arc::clone(&self.inner.preferences),
                route,
                snapshot.selected_effort,
            )
            .await?;
            Ok::<_, ChatRuntimeHostError>(snapshot)
        }
        .await;
        match changed {
            Ok(snapshot) => {
                self.cache_applied_model_controls(project_id, &snapshot)?;
                Ok(snapshot)
            }
            Err(error) => {
                if rollback_model_controls(&slot, &child, &previous)
                    .await
                    .is_ok()
                {
                    return Err(error);
                }
                let retired = retire_child_for_recovery(&slot, &child, chat_id).await;
                drop(operation);
                if retired {
                    let _ = self.open_for_send(chat_id).await;
                }
                Err(ChatRuntimeHostError::InferenceRollbackFailed)
            }
        }
    }

    fn cache_applied_model_controls(
        &self,
        project_id: i64,
        applied: &ModelControlsSnapshot,
    ) -> Result<(), ChatRuntimeHostError> {
        let mut projects = self
            .inner
            .project_environments
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?;
        let cached = projects.get_mut(&project_id).ok_or_else(|| {
            ChatRuntimeHostError::InvalidSessionState(
                "the active chat had no cached model controls".into(),
            )
        })?;
        cached.model_controls.selected_route = applied.selected_route.clone();
        cached.model_controls.selected_effort = applied.selected_effort;
        cached.model_controls.efforts = applied.efforts.clone();
        Ok(())
    }

    async fn effective_model_controls_snapshot(
        &self,
        slot: &ChatSlot,
        child: &PiRpcChild,
        project_id: i64,
    ) -> Result<ModelControlsSnapshot, ChatRuntimeHostError> {
        let allowed = self
            .cached_project_environment(project_id)?
            .ok_or_else(|| {
                ChatRuntimeHostError::InvalidSessionState(
                    "the active chat had no cached model controls".into(),
                )
            })?
            .model_controls;
        self.reconcile_model_controls_snapshot(slot, child, allowed)
            .await
    }

    async fn reconcile_model_controls_snapshot(
        &self,
        slot: &ChatSlot,
        child: &PiRpcChild,
        allowed: ModelControlsSnapshot,
    ) -> Result<ModelControlsSnapshot, ChatRuntimeHostError> {
        let allowed_routes = allowed
            .routes
            .iter()
            .map(|route| route.id.clone())
            .collect::<HashSet<_>>();
        let mut snapshot = model_controls_snapshot(slot, child).await?;

        if !allowed_routes.contains(&snapshot.selected_route) {
            let previous = snapshot;
            let fallback = allowed
                .routes
                .iter()
                .find(|route| route.id == allowed.selected_route)
                .filter(|route| {
                    previous
                        .routes
                        .iter()
                        .any(|candidate| candidate.id == route.id)
                })
                .ok_or_else(|| {
                    ChatRuntimeHostError::InvalidSessionState(
                        "Più's enabled model routes were absent from the active Pi runtime".into(),
                    )
                })?;
            if let Err(error) = child
                .request(
                    serde_json::json!({
                        "type": "set_model",
                        "provider": &fallback.id.provider,
                        "modelId": &fallback.id.model_id,
                    }),
                    CancellationToken::new(),
                )
                .await
            {
                rollback_model_controls(slot, child, &previous).await?;
                return Err(error.into());
            }
            snapshot = match model_controls_snapshot(slot, child).await {
                Ok(applied) if applied.selected_route == fallback.id => applied,
                Ok(_) => {
                    rollback_model_controls(slot, child, &previous).await?;
                    return Err(ChatRuntimeHostError::InferenceChangeRejected);
                }
                Err(error) => {
                    rollback_model_controls(slot, child, &previous).await?;
                    return Err(error);
                }
            };
            let desired_effort = snapshot
                .efforts
                .contains(&allowed.selected_effort)
                .then_some(allowed.selected_effort)
                .or_else(|| snapshot.efforts.first().copied())
                .ok_or_else(|| {
                    ChatRuntimeHostError::InvalidSessionState(
                        "the enabled fallback model exposed no reasoning effort".into(),
                    )
                })?;
            if snapshot.selected_effort != desired_effort {
                if let Err(error) = child
                    .request(
                        serde_json::json!({
                            "type": "set_thinking_level",
                            "level": desired_effort.as_pi(),
                        }),
                        CancellationToken::new(),
                    )
                    .await
                {
                    rollback_model_controls(slot, child, &previous).await?;
                    return Err(error.into());
                }
                snapshot = match model_controls_snapshot(slot, child).await {
                    Ok(applied) if applied.selected_effort == desired_effort => applied,
                    Ok(_) => {
                        rollback_model_controls(slot, child, &previous).await?;
                        return Err(ChatRuntimeHostError::InferenceChangeRejected);
                    }
                    Err(error) => {
                        rollback_model_controls(slot, child, &previous).await?;
                        return Err(error);
                    }
                };
            }
        }

        snapshot
            .routes
            .retain(|route| allowed_routes.contains(&route.id));
        if !snapshot
            .routes
            .iter()
            .any(|route| route.id == snapshot.selected_route)
        {
            return Err(ChatRuntimeHostError::InvalidSessionState(
                "the active model route was disabled in Più".into(),
            ));
        }
        Ok(snapshot)
    }

    pub async fn send(&self, chat_id: &str, text: &str) -> Result<(), ChatRuntimeHostError> {
        self.send_with_attachments(chat_id, text, &[]).await
    }

    pub async fn send_with_attachments(
        &self,
        chat_id: &str,
        text: &str,
        attachments: &[PromptAttachment],
    ) -> Result<(), ChatRuntimeHostError> {
        let text = text.trim();
        if text.is_empty() && attachments.is_empty() {
            return Err(ChatRuntimeHostError::EmptyMessage);
        }
        validate_attachments(attachments)?;
        let delivered_text = prompt_text(text, attachments);
        let command = prompt_command(&delivered_text, attachments, true);
        self.open_for_send(chat_id).await?;
        self.reconcile_pending_inference(chat_id).await?;
        let sent = self
            .send_active(chat_id, &delivered_text, command.clone(), attachments)
            .await;
        if !matches!(sent, Err(ChatRuntimeHostError::NotActive { .. })) {
            return sent;
        }

        self.open_for_send(chat_id).await?;
        self.reconcile_pending_inference(chat_id).await?;
        self.send_active(chat_id, &delivered_text, command, attachments)
            .await
    }

    async fn reconcile_pending_inference(&self, chat_id: &str) -> Result<(), ChatRuntimeHostError> {
        let slot = self.slot(chat_id)?;
        let pending = *slot
            .pending_resource_refresh
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?;
        let running = slot
            .projection
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .snapshot
            .phase
            == ConversationPhase::Running;
        if pending && !running {
            self.model_controls(chat_id).await?;
            *slot
                .pending_resource_refresh
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)? = false;
        }
        Ok(())
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
            &[],
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
        let (changed, tool_events) = {
            let mut projection = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            let changed = projection.snapshot.phase != ConversationPhase::Stopped;
            let mut tool_events = Vec::new();
            interrupt_running_tools(&mut projection, &mut tool_events);
            projection.snapshot.failure = None;
            projection.snapshot.input_request = None;
            projection.snapshot.phase = ConversationPhase::Stopped;
            (changed, tool_events)
        };
        for event in tool_events {
            self.emit(chat_id, event);
        }
        if changed {
            self.emit(chat_id, ConversationEvent::TurnStopped);
        }
        shutdown.map_err(Into::into)
    }

    pub async fn answer_input(
        &self,
        chat_id: &str,
        request_id: &str,
        answer: ConversationInputAnswer,
    ) -> Result<(), ChatRuntimeHostError> {
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
        let pending = slot
            .projection
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .snapshot
            .input_request
            .clone()
            .filter(|pending| pending.id == request_id)
            .ok_or_else(|| ChatRuntimeHostError::InputNotPending {
                chat_id: chat_id.to_owned(),
                request_id: request_id.to_owned(),
            })?;
        let response = match (pending.kind, answer) {
            (_, ConversationInputAnswer::Cancelled) => PiRpcExtensionUiResponse::Cancelled,
            (ConversationInputKind::Select, ConversationInputAnswer::Value { value })
                if pending.options.contains(&value) =>
            {
                PiRpcExtensionUiResponse::Value(value)
            }
            (
                ConversationInputKind::Input | ConversationInputKind::Editor,
                ConversationInputAnswer::Value { value },
            ) => PiRpcExtensionUiResponse::Value(value),
            (ConversationInputKind::Confirm, ConversationInputAnswer::Confirmed { confirmed }) => {
                PiRpcExtensionUiResponse::Confirmed(confirmed)
            }
            _ => return Err(ChatRuntimeHostError::InvalidInputAnswer),
        };
        child.respond_extension_ui(request_id, response).await?;
        let resolved = {
            let mut projection = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            if projection
                .snapshot
                .input_request
                .as_ref()
                .is_some_and(|request| request.id == request_id)
            {
                projection.snapshot.input_request = None;
                true
            } else {
                false
            }
        };
        if resolved {
            self.emit(
                chat_id,
                ConversationEvent::InputResolved {
                    request_id: request_id.to_owned(),
                },
            );
        }
        Ok(())
    }

    async fn send_active(
        &self,
        chat_id: &str,
        text: &str,
        command: Value,
        attachments: &[PromptAttachment],
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
        let has_images = attachments.iter().any(|attachment| {
            attachment.kind == crate::prompt_attachments::PromptAttachmentKind::Image
        });
        if has_images {
            match child_accepts_images(&child).await {
                Ok(true) => {}
                Ok(false) => {
                    retire_send_only_child(&slot, &child).await?;
                    return Err(PromptAttachmentError::ModelMediaUnsupported.into());
                }
                Err(error) => {
                    retire_send_only_child(&slot, &child).await?;
                    return Err(error);
                }
            }
        }
        if let Some(active) = slot
            .active
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .as_mut()
            .filter(|active| Arc::ptr_eq(&active.child, &child))
        {
            active.send_only = false;
        }
        let (expected_index, was_running) = {
            let projection = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            (
                projection.next_message_index,
                projection.snapshot.phase == ConversationPhase::Running,
            )
        };
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
                    id: item_id.clone(),
                    queued: was_running,
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
            let item_id = item.id().to_owned();
            self.emit(
                chat_id,
                ConversationEvent::ItemAdded {
                    before_item_id: None,
                    item,
                },
            );
            if was_running {
                self.emit(
                    chat_id,
                    ConversationEvent::MessageQueueChanged {
                        item_id,
                        queued: true,
                    },
                );
            }
        }
        if let Err(error) = child.request(command, CancellationToken::new()).await {
            let retired = {
                let mut active = slot.active.lock().map_err(|_| ChatRuntimeHostError::Lock)?;
                let owns_command = active.as_ref().is_some_and(|active| {
                    Arc::ptr_eq(&active.child, &child)
                        && active.command_generation == command_generation
                });
                if !owns_command {
                    None
                } else if was_running {
                    if let Some(active) = active.as_mut() {
                        active.command_generation = active.command_generation.wrapping_add(1);
                    }
                    None
                } else {
                    let retired = active.take();
                    if let Some(retired) = &retired {
                        retired.stop_events.cancel();
                    }
                    retired
                }
            };
            if let Some(retired) = retired
                && let Err(shutdown_error) = retired.child.shutdown().await
            {
                tracing::warn!(
                    %shutdown_error,
                    %chat_id,
                    "could not retire chat runtime after prompt rejection"
                );
            }
            let (removed, failed) = {
                let mut projection = slot
                    .projection
                    .lock()
                    .map_err(|_| ChatRuntimeHostError::Lock)?;
                let pending_index = projection
                    .pending_user_items
                    .iter()
                    .position(|(pending_id, _)| pending_id == &item_id);
                let removed = if let Some(pending_index) = pending_index {
                    projection.pending_user_items.remove(pending_index);
                    if let Some(item_index) = projection
                        .snapshot
                        .items
                        .iter()
                        .position(|item| item.id() == item_id)
                    {
                        projection.snapshot.items.remove(item_index);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                let failed = if was_running {
                    false
                } else {
                    projection.snapshot.failure = Some(
                        "Pi couldn’t accept that message. The conversation is still available."
                            .into(),
                    );
                    projection.snapshot.phase = ConversationPhase::Failed;
                    true
                };
                (removed, failed)
            };
            if removed {
                self.emit(
                    chat_id,
                    ConversationEvent::ItemRemoved {
                        item_id: item_id.clone(),
                    },
                );
            }
            if failed {
                self.emit(
                    chat_id,
                    ConversationEvent::TurnFailed {
                        message:
                            "Pi couldn’t accept that message. The conversation is still available."
                                .into(),
                    },
                );
            }
            return Err(error.into());
        }
        Ok(())
    }

    pub async fn open(&self, chat_id: &str) -> Result<ConversationSnapshot, ChatRuntimeHostError> {
        self.open_slot(chat_id, false, true).await
    }

    async fn open_for_send(
        &self,
        chat_id: &str,
    ) -> Result<ConversationSnapshot, ChatRuntimeHostError> {
        self.open_slot(chat_id, true, true).await
    }

    async fn open_slot(
        &self,
        chat_id: &str,
        keep_stopped_runtime: bool,
        inspect_environment_on_miss: bool,
    ) -> Result<ConversationSnapshot, ChatRuntimeHostError> {
        let slot = self.slot(chat_id)?;
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
        let (context, launch_resources, launch_model_controls) = self
            .prepare_launch(chat_id, inspect_environment_on_miss)
            .await?;
        *slot
            .project_id
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)? = Some(context.project_id);

        let _operation = slot.operation.lock().await;
        if !self.is_current_slot(chat_id, &slot)? {
            return Err(ChatWorkspaceError::UnsafeDeletion.into());
        }
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
            chat_id,
            &context.worktree_path,
            stored_session
                .as_ref()
                .map(|session| session.path.as_path()),
            &launch_resources,
            &launch_model_controls,
        )?;
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
            tokio::task::spawn_blocking(move || inbox.initial_prompt(&owned_chat_id))
                .await
                .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))??;
        validate_attachments(&initial_prompt.attachments)?;
        let initial_text = prompt_text(&initial_prompt.text, &initial_prompt.attachments);
        let has_initial_images = initial_prompt.attachments.iter().any(|attachment| {
            attachment.kind == crate::prompt_attachments::PromptAttachmentKind::Image
        });
        if has_initial_images && !child_accepts_images(&child).await? {
            let _ = child.shutdown().await;
            return Err(PromptAttachmentError::ModelMediaUnsupported.into());
        }
        let session_is_empty = messages.is_empty();
        let should_start = stored_session.is_none() && session_is_empty;
        if !session_is_empty && first_user_text(&messages).as_deref() != Some(initial_text.as_str())
        {
            let _ = child.shutdown().await;
            return Err(ChatRuntimeHostError::InvalidSessionState(
                "the exact Pi session did not contain the chat's first message".into(),
            ));
        }

        let mut items = conversation_items(&messages);
        let restored_interruption = interrupt_restored_tools(&mut items);
        if session_is_empty {
            items.push(ConversationItem::Message {
                id: "message-0".into(),
                queued: false,
                role: ConversationRole::User,
                text: initial_text.clone(),
            });
        }
        let snapshot = ConversationSnapshot {
            failure: restored_interruption.then(|| RESTORED_INTERRUPTED_TURN_MESSAGE.to_owned()),
            input_request: None,
            items,
            phase: if restored_interruption {
                ConversationPhase::Interrupted
            } else if should_start {
                ConversationPhase::Running
            } else if stored_session.is_some() {
                if persisted_turn_completed(&messages) {
                    ConversationPhase::Idle
                } else {
                    ConversationPhase::Stopped
                }
            } else {
                ConversationPhase::Idle
            },
            revision: 0,
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
        let snapshot = {
            let mut projection = slot
                .projection
                .lock()
                .map_err(|_| ChatRuntimeHostError::Lock)?;
            let installed = projection.install_opened_session(
                snapshot,
                next_message_index,
                keep_stopped_runtime,
            );
            if installed && should_start {
                projection
                    .pending_user_items
                    .push_back(("message-0".into(), initial_text.clone()));
            }
            projection.snapshot.clone()
        };
        if !should_start && !keep_stopped_runtime {
            child.shutdown().await?;
            return Ok(snapshot);
        }

        let stop_events = CancellationToken::new();
        let rpc_events = child.subscribe();
        *slot.active.lock().map_err(|_| ChatRuntimeHostError::Lock)? = Some(ActiveChat {
            child: Arc::clone(&child),
            command_generation: 0,
            launch_resources,
            project_id: context.project_id,
            send_only: keep_stopped_runtime && !should_start,
            stop_events: stop_events.clone(),
            worktree_path: context.worktree_path.clone(),
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
                    prompt_command(&initial_text, &initial_prompt.attachments, true),
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

    pub async fn retire_for_deletion(&self, chat_id: &str) -> Result<(), ChatRuntimeHostError> {
        let slot = self.slot(chat_id)?;
        let operation = slot.operation.lock().await;
        let active = slot
            .active
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .take();
        if let Some(active) = active {
            active.stop_events.cancel();
            active.child.shutdown().await?;
        }
        self.inner
            .slots
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)?
            .remove(chat_id);
        drop(operation);
        Ok(())
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
        let mut tool_events = Vec::new();
        interrupt_running_tools(&mut projection, &mut tool_events);
        projection.snapshot.failure = None;
        projection.snapshot.input_request = None;
        projection.snapshot.phase = ConversationPhase::Stopped;
        drop(projection);
        for event in tool_events {
            self.emit(chat_id, event);
        }
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
        let Ok(slot) = self.slot(chat_id) else {
            return;
        };
        let Ok(mut projection) = slot.projection.lock() else {
            return;
        };
        let Some((revision, event)) = projection.stamp_events(vec![event]).pop() else {
            return;
        };
        drop(projection);
        let _ = self.inner.events.send(ChatRuntimeChangedEvent {
            chat_id: chat_id.to_owned(),
            event,
            revision,
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
                        let operation = slot.operation.lock().await;
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
                            let projected = projection.stamp_events(projected);
                            let active = active.take();
                            (projected, active)
                        };
                        if let Some(active) = projected.1 {
                            active.stop_events.cancel();
                            if let Err(error) = active.child.shutdown().await {
                                tracing::warn!(%error, %chat_id, "could not retire completed chat runtime");
                            }
                        }
                        for (revision, event) in projected.0 {
                            let _ = inner.events.send(ChatRuntimeChangedEvent {
                                chat_id: chat_id.clone(),
                                event,
                                revision,
                            });
                        }
                        let refresh_resources = slot
                            .pending_resource_refresh
                            .lock()
                            .map(|mut pending| std::mem::take(&mut *pending))
                            .unwrap_or(false);
                        drop(operation);
                        if refresh_resources {
                            let host = ChatRuntimeHost {
                                inner: Arc::clone(&inner),
                            };
                            if let Err(error) = host.open_for_send(&chat_id).await {
                                tracing::warn!(
                                    %error,
                                    %chat_id,
                                    "could not resume chat after applying its deferred resources"
                                );
                            } else if let Err(error) = host.model_controls(&chat_id).await {
                                tracing::warn!(
                                    %error,
                                    %chat_id,
                                    "could not reconcile chat inference after its deferred refresh"
                                );
                                if let Ok(mut pending) = slot.pending_resource_refresh.lock() {
                                    *pending = true;
                                }
                            }
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
                            let projected = project_pi_event(&mut projection, &event.payload);
                            projection.stamp_events(projected)
                        };
                        for (revision, event) in projected {
                            let _ = inner.events.send(ChatRuntimeChangedEvent {
                                chat_id: chat_id.clone(),
                                event,
                                revision,
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
                        tracing::warn!(%error, %chat_id, "Pi runtime stopped during an active turn");
                        let message = RUNTIME_INTERRUPTED_TURN_MESSAGE.to_owned();
                        let projected = if let Ok(mut projection) = slot.projection.lock() {
                            let mut projected = Vec::new();
                            interrupt_running_tools(&mut projection, &mut projected);
                            projection.snapshot.failure = Some(message.clone());
                            clear_pending_input(&mut projection, &mut projected);
                            projection.snapshot.phase = ConversationPhase::Interrupted;
                            projected.push(ConversationEvent::TurnInterrupted { message });
                            projection.stamp_events(projected)
                        } else {
                            return;
                        };
                        if let Err(shutdown_error) = child.shutdown().await {
                            tracing::debug!(
                                %shutdown_error,
                                %chat_id,
                                "failed Pi runtime was already unavailable during retirement"
                            );
                        }
                        for (revision, event) in projected {
                            let _ = inner.events.send(ChatRuntimeChangedEvent {
                                chat_id: chat_id.clone(),
                                event,
                                revision,
                            });
                        }
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

    fn is_current_slot(
        &self,
        chat_id: &str,
        expected: &Arc<ChatSlot>,
    ) -> Result<bool, ChatRuntimeHostError> {
        self.inner
            .slots
            .lock()
            .map_err(|_| ChatRuntimeHostError::Lock)
            .map(|slots| {
                slots
                    .get(chat_id)
                    .is_some_and(|current| Arc::ptr_eq(current, expected))
            })
    }

    fn process_spec(
        &self,
        chat_id: &str,
        worktree: &Path,
        session_path: Option<&Path>,
        launch_resources: &AgentLaunchResources,
        model_controls: &ModelControlsSnapshot,
    ) -> Result<PiRpcProcessSpec, ChatRuntimeHostError> {
        let initial_selection = self.inner.preferences.initial_chat_selection(chat_id)?;
        let (model_provider, model_id, thinking_level) = initial_selection
            .filter(|selection| {
                model_controls.routes.iter().any(|route| {
                    route.id.provider == selection.route.provider_id()
                        && route.id.model_id == selection.route.model_id()
                })
            })
            .map(|selection| {
                (
                    selection.route.provider_id().to_owned(),
                    selection.route.model_id().to_owned(),
                    selection
                        .effort
                        .unwrap_or_else(|| THINKING_LEVEL.to_owned()),
                )
            })
            .unwrap_or_else(|| {
                (
                    model_controls.selected_route.provider.clone(),
                    model_controls.selected_route.model_id.clone(),
                    model_controls.selected_effort.as_pi().to_owned(),
                )
            });
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
            flag(&model_provider),
            flag("--model-id"),
            flag(&model_id),
            flag("--thinking-level"),
            flag(&thinking_level),
        ];
        let mut added_extensions = HashSet::new();
        for extension_path in &launch_resources.extension_paths {
            if added_extensions.insert(extension_path.clone()) {
                arguments.push(flag("--extension"));
                arguments.push(extension_path.as_os_str().to_owned());
            }
        }
        let mut added_skills = HashSet::new();
        if self.inner.paths.app_skill_directory.is_dir() {
            let app_skill_directory = self
                .inner
                .paths
                .app_skill_directory
                .canonicalize()
                .map_err(ChatRuntimeHostError::RuntimeStorage)?;
            if added_skills.insert(app_skill_directory.clone()) {
                arguments.push(flag("--skill"));
                arguments.push(app_skill_directory.into_os_string());
            }
        }
        for skill_path in &launch_resources.skill_paths {
            if added_skills.insert(skill_path.clone()) {
                arguments.push(flag("--skill"));
                arguments.push(skill_path.as_os_str().to_owned());
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
        Ok(PiRpcProcessSpec {
            executable: self.inner.paths.node.clone(),
            arguments,
            working_directory: worktree.to_path_buf(),
            environment,
        })
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
        child
            .request(
                serde_json::json!({ "type": "set_auto_retry", "enabled": false }),
                CancellationToken::new(),
            )
            .await?;
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

async fn remap_launch_resources(
    canonical_resources: &AgentLaunchResources,
    project_root: &Path,
    worktree: &Path,
) -> Result<AgentLaunchResources, ChatRuntimeHostError> {
    let canonical_worktree = tokio::fs::canonicalize(worktree)
        .await
        .map_err(|_| AgentEnvironmentError::InvalidSnapshot)?;
    let mut extension_paths = Vec::with_capacity(canonical_resources.extension_paths.len());
    for path in &canonical_resources.extension_paths {
        extension_paths.push(remap_launch_path(path, project_root, &canonical_worktree).await?);
    }
    let mut skill_paths = Vec::with_capacity(canonical_resources.skill_paths.len());
    for path in &canonical_resources.skill_paths {
        skill_paths.push(remap_launch_path(path, project_root, &canonical_worktree).await?);
    }
    Ok(AgentLaunchResources {
        extension_paths,
        skill_paths,
    })
}

async fn remap_launch_path(
    path: &Path,
    project_root: &Path,
    canonical_worktree: &Path,
) -> Result<PathBuf, ChatRuntimeHostError> {
    let project_relative = path.strip_prefix(project_root).ok();
    let candidate = project_relative
        .map(|relative| canonical_worktree.join(relative))
        .unwrap_or_else(|| path.to_path_buf());
    let canonical = tokio::fs::canonicalize(candidate)
        .await
        .map_err(|_| AgentEnvironmentError::InvalidSnapshot)?;
    if project_relative.is_some() && !canonical.starts_with(canonical_worktree) {
        return Err(AgentEnvironmentError::InvalidSnapshot.into());
    }
    Ok(canonical)
}

fn flag(value: &str) -> OsString {
    OsStr::new(value).to_owned()
}

fn prompt_command(message: &str, attachments: &[PromptAttachment], steer: bool) -> Value {
    let mut command = serde_json::json!({
        "type": "prompt",
        "message": message,
    });
    if steer {
        command["streamingBehavior"] = Value::String("steer".into());
    }
    let images = image_payloads(attachments);
    if !images.is_empty() {
        command["images"] = Value::Array(images);
    }
    command
}

async fn retire_send_only_child(
    slot: &ChatSlot,
    child: &Arc<PiRpcChild>,
) -> Result<(), ChatRuntimeHostError> {
    let active = {
        let mut active = slot.active.lock().map_err(|_| ChatRuntimeHostError::Lock)?;
        if active
            .as_ref()
            .is_some_and(|active| active.send_only && Arc::ptr_eq(&active.child, child))
        {
            active.take()
        } else {
            None
        }
    };
    if let Some(active) = active {
        active.stop_events.cancel();
        if let Err(error) = active.child.shutdown().await {
            tracing::warn!(%error, "could not retire send-only chat runtime after preflight failure");
        }
    }
    Ok(())
}

async fn retire_child_for_recovery(slot: &ChatSlot, child: &PiRpcChild, chat_id: &str) -> bool {
    let active = slot.active.lock().ok().and_then(|mut active| {
        active
            .as_ref()
            .is_some_and(|active| std::ptr::eq(active.child.as_ref(), child))
            .then(|| active.take())
            .flatten()
    });
    let Some(active) = active else {
        return false;
    };
    active.stop_events.cancel();
    if let Err(error) = active.child.shutdown().await {
        tracing::warn!(%error, %chat_id, "could not retire chat after failed inference rollback");
        return false;
    }
    true
}

fn active_model_context(
    slot: &ChatSlot,
    chat_id: &str,
) -> Result<(Arc<PiRpcChild>, i64), ChatRuntimeHostError> {
    slot.active
        .lock()
        .map_err(|_| ChatRuntimeHostError::Lock)?
        .as_ref()
        .map(|active| (Arc::clone(&active.child), active.project_id))
        .ok_or_else(|| ChatRuntimeHostError::NotActive {
            chat_id: chat_id.to_owned(),
        })
}

async fn model_controls_snapshot(
    slot: &ChatSlot,
    child: &PiRpcChild,
) -> Result<ModelControlsSnapshot, ChatRuntimeHostError> {
    let routes = child
        .request(
            serde_json::json!({ "type": "get_available_models" }),
            CancellationToken::new(),
        )
        .await?
        .data
        .and_then(|data| data.get("models").and_then(Value::as_array).cloned())
        .ok_or_else(|| {
            ChatRuntimeHostError::InvalidSessionState(
                "get_available_models omitted the model routes".into(),
            )
        })?
        .into_iter()
        .map(model_route_summary)
        .collect::<Result<Vec<_>, _>>()?;
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
    let selected_route = state
        .get("model")
        .map(model_route_id)
        .transpose()?
        .ok_or_else(|| {
            ChatRuntimeHostError::InvalidSessionState("get_state omitted its model route".into())
        })?;
    if !routes.iter().any(|route| route.id == selected_route) {
        return Err(ChatRuntimeHostError::InvalidSessionState(
            "the active model route was absent from Pi's available models".into(),
        ));
    }
    let selected_effort = state
        .get("thinkingLevel")
        .and_then(Value::as_str)
        .and_then(ReasoningEffort::from_pi)
        .ok_or_else(|| {
            ChatRuntimeHostError::InvalidSessionState(
                "get_state omitted its effective reasoning effort".into(),
            )
        })?;
    let efforts = child
        .request(
            serde_json::json!({ "type": "get_available_thinking_levels" }),
            CancellationToken::new(),
        )
        .await?
        .data
        .and_then(|data| data.get("levels").and_then(Value::as_array).cloned())
        .ok_or_else(|| {
            ChatRuntimeHostError::InvalidSessionState(
                "get_available_thinking_levels omitted the effective levels".into(),
            )
        })?
        .into_iter()
        .map(|level| {
            level
                .as_str()
                .and_then(ReasoningEffort::from_pi)
                .ok_or_else(|| {
                    ChatRuntimeHostError::InvalidSessionState(
                        "Pi returned an unknown reasoning effort".into(),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !efforts.contains(&selected_effort) {
        return Err(ChatRuntimeHostError::InvalidSessionState(
            "the effective reasoning effort was unavailable for the active model".into(),
        ));
    }
    let applies_after_current_step = slot
        .projection
        .lock()
        .map_err(|_| ChatRuntimeHostError::Lock)?
        .snapshot
        .phase
        == ConversationPhase::Running;
    Ok(ModelControlsSnapshot {
        routes,
        selected_route,
        efforts,
        selected_effort,
        applies_after_current_step,
    })
}

fn model_route_summary(model: Value) -> Result<ModelRouteSummary, ChatRuntimeHostError> {
    let id = model_route_id(&model)?;
    let name = model
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(&id.model_id)
        .to_owned();
    let accepts_images = model
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|inputs| inputs.iter().any(|input| input.as_str() == Some("image")));
    Ok(ModelRouteSummary {
        id,
        name,
        accepts_images,
    })
}

fn model_route_id(model: &Value) -> Result<ModelRouteId, ChatRuntimeHostError> {
    let provider = model
        .get("provider")
        .and_then(Value::as_str)
        .filter(|provider| !provider.is_empty())
        .ok_or_else(|| {
            ChatRuntimeHostError::InvalidSessionState("a Pi model omitted its provider".into())
        })?;
    let model_id = model
        .get("id")
        .and_then(Value::as_str)
        .filter(|model_id| !model_id.is_empty())
        .ok_or_else(|| {
            ChatRuntimeHostError::InvalidSessionState("a Pi model omitted its id".into())
        })?;
    Ok(ModelRouteId {
        provider: provider.to_owned(),
        model_id: model_id.to_owned(),
    })
}

async fn rollback_model_controls(
    slot: &ChatSlot,
    child: &PiRpcChild,
    previous: &ModelControlsSnapshot,
) -> Result<(), ChatRuntimeHostError> {
    let route = &previous.selected_route;
    let route_restored = child
        .request(
            serde_json::json!({
                "type": "set_model",
                "provider": &route.provider,
                "modelId": &route.model_id,
            }),
            CancellationToken::new(),
        )
        .await
        .is_ok();
    let effort_restored = child
        .request(
            serde_json::json!({
                "type": "set_thinking_level",
                "level": previous.selected_effort.as_pi(),
            }),
            CancellationToken::new(),
        )
        .await
        .is_ok();
    let restored = model_controls_snapshot(slot, child).await;
    if route_restored
        && effort_restored
        && restored.is_ok_and(|snapshot| {
            snapshot.selected_route == previous.selected_route
                && snapshot.selected_effort == previous.selected_effort
        })
    {
        Ok(())
    } else {
        Err(ChatRuntimeHostError::InferenceRollbackFailed)
    }
}

async fn remembered_effort(
    preferences: Arc<RuntimePreferences>,
    route: PersistedModelRoute,
) -> Result<Option<ReasoningEffort>, ChatRuntimeHostError> {
    tokio::task::spawn_blocking(move || {
        preferences
            .remembered_effort(&route)
            .map(|effort| effort.as_deref().and_then(ReasoningEffort::from_pi))
    })
    .await
    .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))?
    .map_err(Into::into)
}

async fn persist_selected_route(
    preferences: Arc<RuntimePreferences>,
    route: PersistedModelRoute,
    effort: ReasoningEffort,
) -> Result<(), ChatRuntimeHostError> {
    tokio::task::spawn_blocking(move || {
        preferences.select_route_with_effort(&route, effort.as_pi())?;
        Ok::<_, RuntimePreferencesError>(())
    })
    .await
    .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))?
    .map_err(Into::into)
}

async fn persist_effort(
    preferences: Arc<RuntimePreferences>,
    route: PersistedModelRoute,
    effort: ReasoningEffort,
) -> Result<(), ChatRuntimeHostError> {
    tokio::task::spawn_blocking(move || preferences.remember_effort(&route, effort.as_pi()))
        .await
        .map_err(|error| ChatRuntimeHostError::InvalidSessionState(error.to_string()))?
        .map_err(Into::into)
}

async fn child_accepts_images(child: &PiRpcChild) -> Result<bool, ChatRuntimeHostError> {
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
    Ok(state
        .get("model")
        .and_then(|model| model.get("input"))
        .and_then(Value::as_array)
        .is_some_and(|inputs| inputs.iter().any(|input| input.as_str() == Some("image"))))
}

fn is_terminal_pi_event(event: &Value) -> bool {
    match event.get("type").and_then(Value::as_str) {
        Some("auto_retry_start") => true,
        Some("agent_end") => event.get("willRetry").and_then(Value::as_bool) != Some(true),
        _ => false,
    }
}

fn first_user_text(messages: &[Value]) -> Option<String> {
    messages.iter().find_map(|message| {
        (message.get("role").and_then(Value::as_str) == Some("user"))
            .then(|| message_content_text(message.get("content")))
            .flatten()
    })
}

fn persisted_turn_completed(messages: &[Value]) -> bool {
    messages
        .iter()
        .rev()
        .find(|message| {
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("user" | "assistant")
            )
        })
        .is_some_and(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && !matches!(
                    message.get("stopReason").and_then(Value::as_str),
                    Some("aborted" | "error")
                )
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
                        queued: false,
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

fn interrupt_restored_tools(items: &mut [ConversationItem]) -> bool {
    let mut interrupted = false;
    for item in items {
        if let ConversationItem::Tool { status, .. } = item
            && *status == ConversationToolStatus::Running
        {
            *status = ConversationToolStatus::Interrupted;
            interrupted = true;
        }
    }
    interrupted
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
        Some("extension_ui_request") => {
            if let Some(request) = conversation_input_request(event) {
                projection.snapshot.input_request = Some(request.clone());
                emitted.push(ConversationEvent::InputRequested { request });
            }
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
                        if let Some((item_id, _)) = projection.pending_user_items.pop_front() {
                            set_message_queued(projection, &item_id, false, &mut emitted);
                        }
                        return emitted;
                    }
                    projection.pending_user_items.clear();
                    let item = ConversationItem::Message {
                        id: format!("message-{}", projection.next_message_index),
                        queued: false,
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
                                queued: false,
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
        Some("queue_update") => {
            projection.native_steering_count = event
                .get("steering")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            reconcile_native_steering_queue(projection, Some(&mut emitted));
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
        Some("auto_retry_start") => {
            let cause = event
                .get("errorMessage")
                .and_then(Value::as_str)
                .unwrap_or("The model request failed.");
            let message = format!(
                "Pi attempted an automatic retry after this turn failed: {cause} Più stopped the retry; no message was replayed."
            );
            interrupt_running_tools(projection, &mut emitted);
            projection.snapshot.failure = Some(message.clone());
            projection.snapshot.phase = ConversationPhase::Failed;
            projection.active_assistant_index = None;
            projection.tool_content_ids.clear();
            emitted.push(ConversationEvent::TurnFailed { message });
        }
        Some("agent_end") if event.get("willRetry").and_then(Value::as_bool) == Some(true) => {}
        Some("agent_end") => {
            clear_pending_input(projection, &mut emitted);
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
                    let message = "The agent turn was interrupted.".to_owned();
                    interrupt_running_tools(projection, &mut emitted);
                    projection.snapshot.failure = Some(message.clone());
                    projection.snapshot.phase = ConversationPhase::Interrupted;
                    emitted.push(ConversationEvent::TurnInterrupted { message });
                }
                Some("error") => {
                    let message = failure
                        .and_then(|message| message.get("errorMessage"))
                        .and_then(Value::as_str)
                        .unwrap_or("The agent turn failed.")
                        .to_owned();
                    projection.snapshot.failure = Some(message.clone());
                    projection.snapshot.phase = ConversationPhase::Failed;
                    interrupt_running_tools(projection, &mut emitted);
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

fn conversation_input_request(event: &Value) -> Option<ConversationInputRequest> {
    let id = event.get("id")?.as_str()?.to_owned();
    let method = event.get("method")?.as_str()?;
    let title = event
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let kind = match method {
        "select" => ConversationInputKind::Select,
        "confirm" => ConversationInputKind::Confirm,
        "input" => ConversationInputKind::Input,
        "editor" => ConversationInputKind::Editor,
        _ => return None,
    };
    let options: Vec<String> = event
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if kind == ConversationInputKind::Select && options.is_empty() {
        return None;
    }
    Some(ConversationInputRequest {
        id,
        kind,
        title,
        message: event
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        options,
        placeholder: event
            .get("placeholder")
            .and_then(Value::as_str)
            .map(str::to_owned),
        prefill: event
            .get("prefill")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn clear_pending_input(
    projection: &mut ConversationProjection,
    emitted: &mut Vec<ConversationEvent>,
) {
    if let Some(request) = projection.snapshot.input_request.take() {
        emitted.push(ConversationEvent::InputResolved {
            request_id: request.id,
        });
    }
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
    let before_item_id = projection.snapshot.items.iter().find_map(|existing| {
        matches!(
            existing,
            ConversationItem::Message {
                queued: true,
                role: ConversationRole::User,
                ..
            }
        )
        .then(|| existing.id().to_owned())
    });
    if let Some(before_item_id) = &before_item_id
        && let Some(index) = projection
            .snapshot
            .items
            .iter()
            .position(|existing| existing.id() == before_item_id)
    {
        projection.snapshot.items.insert(index, item.clone());
    } else {
        projection.snapshot.items.push(item.clone());
    }
    emitted.push(ConversationEvent::ItemAdded {
        before_item_id,
        item,
    });
}

fn reconcile_native_steering_queue(
    projection: &mut ConversationProjection,
    mut emitted: Option<&mut Vec<ConversationEvent>>,
) {
    let queued_start = projection
        .pending_user_items
        .len()
        .saturating_sub(projection.native_steering_count);
    let desired = projection
        .pending_user_items
        .iter()
        .enumerate()
        .filter_map(|(index, (item_id, _))| (index >= queued_start).then_some(item_id.as_str()))
        .collect::<std::collections::HashSet<_>>();
    for item in &mut projection.snapshot.items {
        let ConversationItem::Message {
            id,
            queued,
            role: ConversationRole::User,
            ..
        } = item
        else {
            continue;
        };
        let next = desired.contains(id.as_str());
        if *queued == next {
            continue;
        }
        *queued = next;
        if let Some(events) = emitted.as_deref_mut() {
            events.push(ConversationEvent::MessageQueueChanged {
                item_id: id.clone(),
                queued: next,
            });
        }
    }
}

fn set_message_queued(
    projection: &mut ConversationProjection,
    item_id: &str,
    queued: bool,
    emitted: &mut Vec<ConversationEvent>,
) {
    let Some(ConversationItem::Message {
        queued: current, ..
    }) = projection
        .snapshot
        .items
        .iter_mut()
        .find(|item| item.id() == item_id)
    else {
        return;
    };
    if *current == queued {
        return;
    }
    *current = queued;
    emitted.push(ConversationEvent::MessageQueueChanged {
        item_id: item_id.to_owned(),
        queued,
    });
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

fn interrupt_running_tools(
    projection: &mut ConversationProjection,
    emitted: &mut Vec<ConversationEvent>,
) {
    for item in &mut projection.snapshot.items {
        let ConversationItem::Tool {
            detail, id, status, ..
        } = item
        else {
            continue;
        };
        if *status != ConversationToolStatus::Running {
            continue;
        }
        *status = ConversationToolStatus::Interrupted;
        emitted.push(ConversationEvent::ToolUpdate {
            detail: detail.clone(),
            item_id: id.clone(),
            status: ConversationToolStatus::Interrupted,
        });
    }
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
                    queued: false,
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

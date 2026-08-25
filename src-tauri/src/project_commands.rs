use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, State};
use ts_rs::TS;

use crate::{
    agent_environment::AgentEnvironment,
    application::ApplicationCore,
    chat_runtime_host::{ModelRouteId, ReasoningEffort},
    chat_workspaces::{
        ChatSetupChangedEvent, ChatTerminalRequest, ChatWorkspaceError, CreatedChat,
    },
    project_inbox::{DraftSummary, InboxSnapshot, OpenRepositoryOutcome, ProjectInboxError},
    prompt_attachments::PromptAttachment,
};

pub const PROJECT_INBOX_CHANGED_EVENT: &str = "project-inbox://changed";
pub const CHAT_SETUP_CHANGED_EVENT: &str = "chat-workspace://setup-changed";
pub const CHAT_TERMINAL_REQUESTED_EVENT: &str = "chat-workspace://terminal-requested";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct OpenRepositoryRequest {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct OpenRepositoryResponse {
    #[ts(type = "number")]
    pub focused_project_id: i64,
    pub outcome: OpenRepositoryOutcome,
    pub snapshot: InboxSnapshot,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct SaveProjectDraftRequest {
    #[ts(type = "number")]
    pub project_id: i64,
    pub prompt: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct RemoveProjectRequest {
    #[ts(type = "number")]
    pub project_id: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct CreateChatRequest {
    #[ts(type = "number")]
    pub project_id: i64,
    pub prompt: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
    pub route: ModelRouteId,
    pub effort: ReasoningEffort,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatIdRequest {
    pub chat_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct RenameChatRequest {
    pub chat_id: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct CreateChatResponse {
    pub chat: crate::project_inbox::ChatSummary,
    pub snapshot: InboxSnapshot,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProjectInboxChangedEvent {
    pub snapshot: InboxSnapshot,
    #[ts(type = "number | null")]
    pub focused_project_id: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ProjectCommandErrorCode {
    InvalidRepository,
    RepositoryInaccessible,
    ProjectHasUnmergedChats,
    ProjectNotFound,
    ChatNotFound,
    InvalidChatTitle,
    RepositoryInspectionFailed,
    StorageUnavailable,
    InvalidAttachment,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProjectCommandError {
    pub code: ProjectCommandErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ChatWorkspaceCommandErrorCode {
    EmptyPrompt,
    ProjectNotFound,
    ChatNotFound,
    FreshMainUnavailable,
    SetupAlreadyRunning,
    CreationFailed,
    StorageUnavailable,
    InvalidAttachment,
    InferenceUnavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatWorkspaceCommandError {
    pub code: ChatWorkspaceCommandErrorCode,
    pub message: String,
}

impl From<ChatWorkspaceError> for ChatWorkspaceCommandError {
    fn from(error: ChatWorkspaceError) -> Self {
        match error {
            ChatWorkspaceError::EmptyPrompt => Self {
                code: ChatWorkspaceCommandErrorCode::EmptyPrompt,
                message: "Write a message before starting the chat.".into(),
            },
            ChatWorkspaceError::Inbox(ProjectInboxError::ProjectNotFound { .. }) => Self {
                code: ChatWorkspaceCommandErrorCode::ProjectNotFound,
                message: "That project is no longer in Più.".into(),
            },
            ChatWorkspaceError::Inbox(ProjectInboxError::ChatNotFound { .. }) => Self {
                code: ChatWorkspaceCommandErrorCode::ChatNotFound,
                message: "That chat is no longer in Più.".into(),
            },
            ChatWorkspaceError::FreshMain(_) => Self {
                code: ChatWorkspaceCommandErrorCode::FreshMainUnavailable,
                message:
                    "Più couldn’t fetch a fresh origin/main. Check remote access and try again."
                        .into(),
            },
            ChatWorkspaceError::SetupAlreadyRunning => Self {
                code: ChatWorkspaceCommandErrorCode::SetupAlreadyRunning,
                message: "Setup is already running for this chat.".into(),
            },
            ChatWorkspaceError::Inbox(ProjectInboxError::Attachment(_)) => Self {
                code: ChatWorkspaceCommandErrorCode::InvalidAttachment,
                message: "One of those attachments is no longer valid. Remove it and try again."
                    .into(),
            },
            ChatWorkspaceError::WorktreeStorage(_)
            | ChatWorkspaceError::Inbox(ProjectInboxError::AppData(_))
            | ChatWorkspaceError::Inbox(ProjectInboxError::Database(_))
            | ChatWorkspaceError::Inbox(ProjectInboxError::DatabaseLock)
            | ChatWorkspaceError::Inbox(ProjectInboxError::SystemClock) => Self {
                code: ChatWorkspaceCommandErrorCode::StorageUnavailable,
                message: "Più couldn’t save this chat. Try again.".into(),
            },
            ChatWorkspaceError::Git(_)
            | ChatWorkspaceError::InvalidOwnership
            | ChatWorkspaceError::Reconciliation(_)
            | ChatWorkspaceError::Interrupted(_)
            | ChatWorkspaceError::SetupSupervisor(_)
            | ChatWorkspaceError::Inbox(_) => Self {
                code: ChatWorkspaceCommandErrorCode::CreationFailed,
                message: "Più couldn’t prepare this chat. Your repository was not changed.".into(),
            },
        }
    }
}

impl From<ProjectInboxError> for ProjectCommandError {
    fn from(error: ProjectInboxError) -> Self {
        match error {
            ProjectInboxError::InvalidRepository => Self {
                code: ProjectCommandErrorCode::InvalidRepository,
                message: "Choose a folder that contains a Git repository.".into(),
            },
            ProjectInboxError::RepositoryInaccessible => Self {
                code: ProjectCommandErrorCode::RepositoryInaccessible,
                message: "Più can’t access that repository. Check its permissions and try again."
                    .into(),
            },
            ProjectInboxError::ProjectHasUnmergedChats { count } => Self {
                code: ProjectCommandErrorCode::ProjectHasUnmergedChats,
                message: if count == 1 {
                    "Merge its active chat before removing the project.".into()
                } else {
                    format!("Merge its {count} active chats before removing the project.")
                },
            },
            ProjectInboxError::ProjectNotFound { .. } => Self {
                code: ProjectCommandErrorCode::ProjectNotFound,
                message: "That project is no longer in Più.".into(),
            },
            ProjectInboxError::ChatNotFound { .. } => Self {
                code: ProjectCommandErrorCode::ChatNotFound,
                message: "That chat is no longer in Più.".into(),
            },
            ProjectInboxError::InvalidChatTitle => Self {
                code: ProjectCommandErrorCode::InvalidChatTitle,
                message: "Give this chat a title before saving.".into(),
            },
            ProjectInboxError::Attachment(_) => Self {
                code: ProjectCommandErrorCode::InvalidAttachment,
                message: "One of those attachments is no longer valid. Remove it and try again."
                    .into(),
            },
            ProjectInboxError::GitProcess(_) => Self {
                code: ProjectCommandErrorCode::RepositoryInspectionFailed,
                message: "Più couldn’t inspect that repository. Try again.".into(),
            },
            ProjectInboxError::AppData(_)
            | ProjectInboxError::Database(_)
            | ProjectInboxError::DatabaseLock
            | ProjectInboxError::ChatSessionAlreadyBound { .. }
            | ProjectInboxError::InvalidChatSessionReference { .. }
            | ProjectInboxError::InvalidAttachmentState
            | ProjectInboxError::SystemClock => Self {
                code: ProjectCommandErrorCode::StorageUnavailable,
                message: "Più couldn’t save this change. Try again.".into(),
            },
        }
    }
}

#[tauri::command]
pub async fn load_project_inbox(
    core: State<'_, ApplicationCore>,
) -> Result<InboxSnapshot, ProjectCommandError> {
    load_project_inbox_from(core.project_inbox(), Some(core.chat_workspaces())).await
}

#[tauri::command]
pub async fn open_repository<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    request: OpenRepositoryRequest,
) -> Result<OpenRepositoryResponse, ProjectCommandError> {
    let inbox = core.project_inbox();
    let selected_path = PathBuf::from(request.path);
    let opened = blocking_project_operation(move || inbox.open_repository(&selected_path)).await?;
    let response = OpenRepositoryResponse {
        focused_project_id: opened.project.id,
        outcome: opened.outcome,
        snapshot: opened.snapshot,
    };
    emit_change(
        &app,
        ProjectInboxChangedEvent {
            snapshot: response.snapshot.clone(),
            focused_project_id: Some(response.focused_project_id),
        },
    )?;
    Ok(response)
}

#[tauri::command]
pub async fn save_project_draft(
    core: State<'_, ApplicationCore>,
    request: SaveProjectDraftRequest,
) -> Result<DraftSummary, ProjectCommandError> {
    let inbox = core.project_inbox();
    blocking_project_operation(move || {
        inbox.save_draft(request.project_id, &request.prompt, &request.attachments)
    })
    .await
}

#[tauri::command]
pub async fn remove_project<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    request: RemoveProjectRequest,
) -> Result<InboxSnapshot, ProjectCommandError> {
    let inbox = core.project_inbox();
    let snapshot =
        blocking_project_operation(move || inbox.remove_project(request.project_id)).await?;
    emit_change(
        &app,
        ProjectInboxChangedEvent {
            snapshot: snapshot.clone(),
            focused_project_id: None,
        },
    )?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn rename_chat<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    request: RenameChatRequest,
) -> Result<InboxSnapshot, ProjectCommandError> {
    let inbox = core.project_inbox();
    let snapshot =
        blocking_project_operation(move || inbox.rename_chat(&request.chat_id, &request.title))
            .await?;
    emit_change(
        &app,
        ProjectInboxChangedEvent {
            snapshot: snapshot.clone(),
            focused_project_id: None,
        },
    )?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn create_chat<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    environment: State<'_, Arc<AgentEnvironment>>,
    request: CreateChatRequest,
) -> Result<CreateChatResponse, ChatWorkspaceCommandError> {
    let selection = environment
        .validate_model_selection(request.project_id, request.route, request.effort)
        .await
        .map_err(|_| ChatWorkspaceCommandError {
            code: ChatWorkspaceCommandErrorCode::InferenceUnavailable,
            message: "That model or reasoning effort is no longer available. Choose again.".into(),
        })?;
    let workspaces = core.chat_workspaces();
    let setup_workspaces = Arc::clone(&workspaces);
    let CreatedChat { chat, snapshot } = blocking_chat_operation(move || {
        workspaces.create_chat(
            request.project_id,
            &request.prompt,
            &request.attachments,
            selection,
        )
    })
    .await?;
    let chat_id = chat.id.clone();

    let setup_app = app.clone();
    let on_change = Arc::new(move |event: ChatSetupChangedEvent| {
        if let Err(error) = setup_app.emit(CHAT_SETUP_CHANGED_EVENT, event) {
            tracing::warn!(%error, "could not emit setup change");
        }
    });
    let setup_chat_id = chat_id.clone();
    if let Err(error) =
        blocking_chat_operation(move || setup_workspaces.start_setup(&setup_chat_id, on_change))
            .await
    {
        tracing::warn!(?error, %chat_id, "could not start chat setup");
    }

    let latest_snapshot = {
        let inbox = core.project_inbox();
        blocking_project_operation(move || inbox.snapshot())
            .await
            .unwrap_or(snapshot)
    };
    let latest_chat = latest_snapshot
        .chats
        .iter()
        .find(|candidate| candidate.id == chat_id)
        .cloned()
        .unwrap_or(chat);
    let response = CreateChatResponse {
        chat: latest_chat,
        snapshot: latest_snapshot,
    };
    emit_change(
        &app,
        ProjectInboxChangedEvent {
            snapshot: response.snapshot.clone(),
            focused_project_id: response.chat.project_id,
        },
    )
    .map_err(|error| ChatWorkspaceCommandError {
        code: ChatWorkspaceCommandErrorCode::StorageUnavailable,
        message: error.message,
    })?;
    Ok(response)
}

#[tauri::command]
pub async fn retry_chat_setup<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    request: ChatIdRequest,
) -> Result<crate::project_inbox::ChatSetupSummary, ChatWorkspaceCommandError> {
    let setup_app = app.clone();
    let on_change = Arc::new(move |event: ChatSetupChangedEvent| {
        if let Err(error) = setup_app.emit(CHAT_SETUP_CHANGED_EVENT, event) {
            tracing::warn!(%error, "could not emit retried setup change");
        }
    });
    let workspaces = core.chat_workspaces();
    blocking_chat_operation(move || workspaces.start_setup(&request.chat_id, on_change)).await
}

#[tauri::command]
pub async fn cancel_chat_setup(
    core: State<'_, ApplicationCore>,
    request: ChatIdRequest,
) -> Result<(), ChatWorkspaceCommandError> {
    core.chat_workspaces()
        .cancel_setup(&request.chat_id)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn open_chat_terminal<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    request: ChatIdRequest,
) -> Result<ChatTerminalRequest, ChatWorkspaceCommandError> {
    let workspaces = core.chat_workspaces();
    let request =
        blocking_chat_operation(move || workspaces.terminal_request(&request.chat_id)).await?;
    app.emit(CHAT_TERMINAL_REQUESTED_EVENT, &request)
        .map_err(|_| ChatWorkspaceCommandError {
            code: ChatWorkspaceCommandErrorCode::StorageUnavailable,
            message: "Più couldn’t open the chat terminal. Try again.".into(),
        })?;
    Ok(request)
}

async fn load_project_inbox_from(
    inbox: Arc<crate::project_inbox::ProjectInbox>,
    workspaces: Option<Arc<crate::chat_workspaces::ChatWorkspaces>>,
) -> Result<InboxSnapshot, ProjectCommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(workspaces) = workspaces {
            workspaces
                .reconcile_once()
                .inspect_err(|error| {
                    tracing::error!(
                        error = ?error,
                        "chat workspace reconciliation failed while loading the inbox"
                    );
                })
                .map_err(ChatWorkspaceCommandError::from)
                .map_err(|error| ProjectCommandError {
                    code: ProjectCommandErrorCode::StorageUnavailable,
                    message: error.message,
                })?;
        }
        inbox
            .snapshot()
            .inspect_err(|error| {
                tracing::error!(error = ?error, "project inbox snapshot failed during startup");
            })
            .map_err(Into::into)
    })
    .await
    .map_err(|_| ProjectCommandError {
        code: ProjectCommandErrorCode::StorageUnavailable,
        message: "Più couldn’t load the inbox. Try again.".into(),
    })?
}

async fn blocking_project_operation<T>(
    operation: impl FnOnce() -> Result<T, ProjectInboxError> + Send + 'static,
) -> Result<T, ProjectCommandError>
where
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| ProjectCommandError {
            code: ProjectCommandErrorCode::StorageUnavailable,
            message: "Più couldn’t finish this operation. Try again.".into(),
        })?
        .map_err(Into::into)
}

async fn blocking_chat_operation<T>(
    operation: impl FnOnce() -> Result<T, ChatWorkspaceError> + Send + 'static,
) -> Result<T, ChatWorkspaceCommandError>
where
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| ChatWorkspaceCommandError {
            code: ChatWorkspaceCommandErrorCode::CreationFailed,
            message: "Più couldn’t finish creating this chat. Try again.".into(),
        })?
        .map_err(Into::into)
}

fn emit_change<R: Runtime>(
    app: &AppHandle<R>,
    event: ProjectInboxChangedEvent,
) -> Result<(), ProjectCommandError> {
    app.emit(PROJECT_INBOX_CHANGED_EVENT, event)
        .map_err(|_| ProjectCommandError {
            code: ProjectCommandErrorCode::StorageUnavailable,
            message: "Più saved the change but couldn’t refresh the inbox. Reopen Più to continue."
                .into(),
        })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        future::Future,
        path::Path,
        process::Command,
        sync::{Arc, Mutex, mpsc},
        task::{Context, Poll, Waker},
        thread,
        time::{Duration, Instant},
    };

    use crate::{
        git_process::GitProcess,
        project_inbox::{
            ProjectInbox, RepositoryIdentity, RepositoryInspectionError, RepositoryInspector,
        },
    };

    struct DelayedInspector {
        started: Mutex<Option<mpsc::SyncSender<()>>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl RepositoryInspector for DelayedInspector {
        fn inspect(
            &self,
            _selected_path: &Path,
        ) -> Result<RepositoryIdentity, RepositoryInspectionError> {
            if let Some(started) = self.started.lock().unwrap().take() {
                started.send(()).unwrap();
            }
            self.release.lock().unwrap().recv().unwrap();
            Err(RepositoryInspectionError::Missing)
        }
    }

    #[test]
    fn delayed_git_inspection_yields_the_ipc_executor() {
        let fixture = tempfile::tempdir().unwrap();
        let repository = fixture.path().join("repository");
        fs::create_dir(&repository).unwrap();
        assert!(
            Command::new("/usr/bin/git")
                .args(["init", "--quiet"])
                .arg(&repository)
                .status()
                .unwrap()
                .success()
        );
        let database_path = fixture.path().join("piu.sqlite3");
        let initial = ProjectInbox::with_git(
            &database_path,
            GitProcess::with_executable("/usr/bin/git".into()),
        )
        .unwrap();
        initial.open_repository(&repository).unwrap();
        drop(initial);

        let (started_sender, started_receiver) = mpsc::sync_channel(0);
        let (release_sender, release_receiver) = mpsc::sync_channel(0);
        let inbox = Arc::new(
            ProjectInbox::with_inspector(
                &database_path,
                Arc::new(DelayedInspector {
                    started: Mutex::new(Some(started_sender)),
                    release: Mutex::new(release_receiver),
                }),
            )
            .unwrap(),
        );
        let mut operation = Box::pin(super::load_project_inbox_from(inbox, None));
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let started = Instant::now();

        assert!(matches!(
            operation.as_mut().poll(&mut context),
            Poll::Pending
        ));
        assert!(started.elapsed() < Duration::from_millis(250));
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking repository inspection should start on its worker");
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            release_sender.send(()).unwrap();
        });

        let snapshot = tauri::async_runtime::block_on(operation).unwrap();
        assert_eq!(snapshot.projects.len(), 1);
    }
}

use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, State};
use ts_rs::TS;

use crate::{
    application::ApplicationCore,
    project_inbox::{DraftSummary, InboxSnapshot, OpenRepositoryOutcome, ProjectInboxError},
};

pub const PROJECT_INBOX_CHANGED_EVENT: &str = "project-inbox://changed";

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
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct RemoveProjectRequest {
    #[ts(type = "number")]
    pub project_id: i64,
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
    RepositoryInspectionFailed,
    StorageUnavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProjectCommandError {
    pub code: ProjectCommandErrorCode,
    pub message: String,
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
            ProjectInboxError::GitProcess(_) => Self {
                code: ProjectCommandErrorCode::RepositoryInspectionFailed,
                message: "Più couldn’t inspect that repository. Try again.".into(),
            },
            ProjectInboxError::AppData(_)
            | ProjectInboxError::Database(_)
            | ProjectInboxError::DatabaseLock
            | ProjectInboxError::SystemClock => Self {
                code: ProjectCommandErrorCode::StorageUnavailable,
                message: "Più couldn’t save this change. Try again.".into(),
            },
        }
    }
}

#[tauri::command]
pub fn load_project_inbox(
    core: State<'_, ApplicationCore>,
) -> Result<InboxSnapshot, ProjectCommandError> {
    core.project_inbox().snapshot().map_err(Into::into)
}

#[tauri::command]
pub fn open_repository<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    request: OpenRepositoryRequest,
) -> Result<OpenRepositoryResponse, ProjectCommandError> {
    let opened = core
        .project_inbox()
        .open_repository(Path::new(&request.path))
        .map_err(ProjectCommandError::from)?;
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
pub fn save_project_draft(
    core: State<'_, ApplicationCore>,
    request: SaveProjectDraftRequest,
) -> Result<DraftSummary, ProjectCommandError> {
    core.project_inbox()
        .save_draft(request.project_id, &request.prompt)
        .map_err(Into::into)
}

#[tauri::command]
pub fn remove_project<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    request: RemoveProjectRequest,
) -> Result<InboxSnapshot, ProjectCommandError> {
    let snapshot = core
        .project_inbox()
        .remove_project(request.project_id)
        .map_err(ProjectCommandError::from)?;
    emit_change(
        &app,
        ProjectInboxChangedEvent {
            snapshot: snapshot.clone(),
            focused_project_id: None,
        },
    )?;
    Ok(snapshot)
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

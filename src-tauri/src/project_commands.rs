use std::{path::PathBuf, sync::Arc};

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
pub async fn load_project_inbox(
    core: State<'_, ApplicationCore>,
) -> Result<InboxSnapshot, ProjectCommandError> {
    load_project_inbox_from(core.project_inbox()).await
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
    blocking_project_operation(move || inbox.save_draft(request.project_id, &request.prompt)).await
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

async fn load_project_inbox_from(
    inbox: Arc<crate::project_inbox::ProjectInbox>,
) -> Result<InboxSnapshot, ProjectCommandError> {
    blocking_project_operation(move || inbox.snapshot()).await
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
        let mut operation = Box::pin(super::load_project_inbox_from(inbox));
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

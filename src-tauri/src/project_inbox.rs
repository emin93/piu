use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::{
    database::{Database, DatabaseError},
    git_process::{GitProcess, GitProcessError},
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ProjectAvailability {
    Available,
    Missing,
    Inaccessible,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ChatMergeState {
    Unmerged,
    Merged,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProjectSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub availability: ProjectAvailability,
    pub unmerged_chat_count: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct DraftSummary {
    #[ts(type = "number")]
    pub project_id: i64,
    pub prompt: String,
    #[ts(type = "number")]
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatSummary {
    pub id: String,
    #[ts(type = "number | null")]
    pub project_id: Option<i64>,
    pub project_name: String,
    pub title: String,
    pub branch_name: String,
    pub pull_request_number: Option<u32>,
    #[ts(type = "number")]
    pub created_at_ms: i64,
    pub merge_state: ChatMergeState,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct InboxSnapshot {
    pub projects: Vec<ProjectSummary>,
    pub drafts: Vec<DraftSummary>,
    pub chats: Vec<ChatSummary>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum OpenRepositoryOutcome {
    Added,
    FocusedExisting,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct OpenRepositoryResult {
    pub outcome: OpenRepositoryOutcome,
    pub project: ProjectSummary,
    pub snapshot: InboxSnapshot,
}

#[derive(Debug, Error)]
pub enum ProjectInboxError {
    #[error("the selected folder is not a Git repository")]
    InvalidRepository,
    #[error("the selected repository cannot be accessed")]
    RepositoryInaccessible,
    #[error("project {project_id} does not exist")]
    ProjectNotFound { project_id: i64 },
    #[error("project has {count} unmerged chats")]
    ProjectHasUnmergedChats { count: u32 },
    #[error("could not prepare application data: {0}")]
    AppData(#[source] std::io::Error),
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("project inbox lock is poisoned")]
    DatabaseLock,
    #[error("system clock is before the Unix epoch")]
    SystemClock,
    #[error(transparent)]
    GitProcess(#[from] GitProcessError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryIdentity {
    canonical_path: PathBuf,
    root_device: String,
    root_inode: String,
    git_dir_path: PathBuf,
    git_dir_device: String,
    git_dir_inode: String,
}

#[derive(Debug, Error)]
pub enum RepositoryInspectionError {
    #[error("the repository is missing")]
    Missing,
    #[error("the repository is inaccessible")]
    Inaccessible,
    #[error(transparent)]
    Git(#[from] GitProcessError),
}

pub trait RepositoryInspector: Send + Sync {
    fn inspect(
        &self,
        selected_path: &Path,
    ) -> Result<RepositoryIdentity, RepositoryInspectionError>;
}

impl RepositoryInspector for GitProcess {
    fn inspect(
        &self,
        selected_path: &Path,
    ) -> Result<RepositoryIdentity, RepositoryInspectionError> {
        match fs::metadata(selected_path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(RepositoryInspectionError::Missing),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(RepositoryInspectionError::Inaccessible);
            }
            Err(_) => return Err(RepositoryInspectionError::Missing),
        }
        let paths = self
            .inspect_worktree(selected_path)
            .map_err(|error| match error {
                GitProcessError::Failed { ref stderr, .. }
                    if stderr.to_ascii_lowercase().contains("permission denied") =>
                {
                    RepositoryInspectionError::Inaccessible
                }
                GitProcessError::Failed { .. } => RepositoryInspectionError::Missing,
                other => RepositoryInspectionError::Git(other),
            })?;
        let canonical_path = paths
            .root
            .canonicalize()
            .map_err(classify_identity_io_error)?;
        let git_dir_path = paths
            .git_dir
            .canonicalize()
            .map_err(classify_identity_io_error)?;
        let root_metadata = fs::metadata(&canonical_path).map_err(classify_identity_io_error)?;
        let git_dir_metadata = fs::metadata(&git_dir_path).map_err(classify_identity_io_error)?;
        Ok(RepositoryIdentity {
            canonical_path,
            root_device: root_metadata.dev().to_string(),
            root_inode: root_metadata.ino().to_string(),
            git_dir_path,
            git_dir_device: git_dir_metadata.dev().to_string(),
            git_dir_inode: git_dir_metadata.ino().to_string(),
        })
    }
}

fn classify_identity_io_error(error: std::io::Error) -> RepositoryInspectionError {
    match error.kind() {
        std::io::ErrorKind::NotFound => RepositoryInspectionError::Missing,
        _ => RepositoryInspectionError::Inaccessible,
    }
}

pub struct ProjectInbox {
    database_path: PathBuf,
    database: Mutex<Option<Database>>,
    repository_inspector: Arc<dyn RepositoryInspector>,
}

impl ProjectInbox {
    pub fn with_git(database_path: &Path, git: GitProcess) -> Result<Self, ProjectInboxError> {
        Self::with_inspector(database_path, Arc::new(git))
    }

    pub fn with_inspector(
        database_path: &Path,
        repository_inspector: Arc<dyn RepositoryInspector>,
    ) -> Result<Self, ProjectInboxError> {
        let inbox = Self {
            database_path: database_path.to_path_buf(),
            database: Mutex::new(None),
            repository_inspector,
        };
        inbox.ensure_storage_ready()?;
        Ok(inbox)
    }

    pub fn deferred_with_git(database_path: PathBuf, git: GitProcess) -> Self {
        Self::deferred_with_inspector(database_path, Arc::new(git))
    }

    pub fn deferred_with_inspector(
        database_path: PathBuf,
        repository_inspector: Arc<dyn RepositoryInspector>,
    ) -> Self {
        Self {
            database_path,
            database: Mutex::new(None),
            repository_inspector,
        }
    }

    pub fn ensure_storage_ready(&self) -> Result<(), ProjectInboxError> {
        self.with_database(|_| Ok(()))
    }

    pub fn open_repository(
        &self,
        selected_path: &Path,
    ) -> Result<OpenRepositoryResult, ProjectInboxError> {
        let identity = self
            .repository_inspector
            .inspect(selected_path)
            .map_err(map_admission_error)?;
        let name = repository_name(&identity.canonical_path)?;
        let canonical_path = identity.canonical_path.to_string_lossy().into_owned();
        let git_dir_path = identity.git_dir_path.to_string_lossy().into_owned();
        let created_at_ms = now_ms()?;

        let (project_id, outcome) = self.with_database(|database| {
            let connection = database.connection_mut();
            let transaction = connection
                .transaction()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            let existing_id = transaction
                .query_row(
                    "SELECT id FROM projects WHERE canonical_path = ?1",
                    [&canonical_path],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            let (project_id, outcome) = match existing_id {
                Some(project_id) => (project_id, OpenRepositoryOutcome::FocusedExisting),
                None => {
                    transaction
                        .execute(
                            "INSERT INTO projects (
                               canonical_path, root_device, root_inode, git_dir_path,
                               git_dir_device, git_dir_inode, name, created_at_ms
                             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                            params![
                                canonical_path,
                                identity.root_device,
                                identity.root_inode,
                                git_dir_path,
                                identity.git_dir_device,
                                identity.git_dir_inode,
                                name,
                                created_at_ms
                            ],
                        )
                        .map_err(DatabaseError::Query)
                        .map_err(ProjectInboxError::Database)?;
                    (
                        transaction.last_insert_rowid(),
                        OpenRepositoryOutcome::Added,
                    )
                }
            };
            transaction
                .commit()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            Ok((project_id, outcome))
        })?;
        let snapshot = self.snapshot()?;
        let project = snapshot
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .cloned()
            .ok_or(ProjectInboxError::ProjectNotFound { project_id })?;
        Ok(OpenRepositoryResult {
            outcome,
            project,
            snapshot,
        })
    }

    pub fn save_draft(
        &self,
        project_id: i64,
        prompt: &str,
    ) -> Result<DraftSummary, ProjectInboxError> {
        let updated_at_ms = now_ms()?;
        self.with_database(|database| {
            let connection = database.connection_mut();
            let transaction = connection
                .transaction()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            require_project(&transaction, project_id)?;
            if prompt.is_empty() {
                transaction
                    .execute(
                        "DELETE FROM chat_drafts WHERE project_id = ?1",
                        [project_id],
                    )
                    .map_err(DatabaseError::Query)
                    .map_err(ProjectInboxError::Database)?;
            } else {
                transaction
                    .execute(
                        "INSERT INTO chat_drafts (project_id, prompt, updated_at_ms)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT(project_id) DO UPDATE SET
                           prompt = excluded.prompt,
                           updated_at_ms = excluded.updated_at_ms",
                        params![project_id, prompt, updated_at_ms],
                    )
                    .map_err(DatabaseError::Query)
                    .map_err(ProjectInboxError::Database)?;
            }
            transaction
                .commit()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            Ok(DraftSummary {
                project_id,
                prompt: prompt.to_owned(),
                updated_at_ms,
            })
        })
    }

    pub fn remove_project(&self, project_id: i64) -> Result<InboxSnapshot, ProjectInboxError> {
        self.with_database(|database| {
            let connection = database.connection_mut();
            let transaction = connection
                .transaction()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            require_project(&transaction, project_id)?;
            let unmerged_count: u32 = transaction
                .query_row(
                    "SELECT COUNT(*) FROM chats
                     WHERE project_id = ?1 AND merge_state = 'unmerged'",
                    [project_id],
                    |row| row.get(0),
                )
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            if unmerged_count > 0 {
                return Err(ProjectInboxError::ProjectHasUnmergedChats {
                    count: unmerged_count,
                });
            }
            transaction
                .execute("DELETE FROM projects WHERE id = ?1", [project_id])
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            transaction
                .commit()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)
        })?;
        self.snapshot()
    }

    pub fn snapshot(&self) -> Result<InboxSnapshot, ProjectInboxError> {
        let stored = self.with_database(|database| load_stored_snapshot(database.connection()))?;
        Ok(stored.materialize(self.repository_inspector.as_ref()))
    }

    fn with_database<T>(
        &self,
        operation: impl FnOnce(&mut Database) -> Result<T, ProjectInboxError>,
    ) -> Result<T, ProjectInboxError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| ProjectInboxError::DatabaseLock)?;
        if database.is_none() {
            if let Some(parent) = self.database_path.parent() {
                fs::create_dir_all(parent).map_err(ProjectInboxError::AppData)?;
            }
            *database = Some(Database::open(&self.database_path)?);
        }
        operation(
            database
                .as_mut()
                .expect("database is initialized before use"),
        )
    }
}

fn map_admission_error(error: RepositoryInspectionError) -> ProjectInboxError {
    match error {
        RepositoryInspectionError::Missing => ProjectInboxError::InvalidRepository,
        RepositoryInspectionError::Inaccessible => ProjectInboxError::RepositoryInaccessible,
        RepositoryInspectionError::Git(error) => ProjectInboxError::GitProcess(error),
    }
}

fn repository_name(canonical_path: &Path) -> Result<String, ProjectInboxError> {
    canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or(ProjectInboxError::InvalidRepository)
}

fn require_project(
    transaction: &Transaction<'_>,
    project_id: i64,
) -> Result<(), ProjectInboxError> {
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
            [project_id],
            |row| row.get(0),
        )
        .map_err(DatabaseError::Query)
        .map_err(ProjectInboxError::Database)?;
    if exists {
        Ok(())
    } else {
        Err(ProjectInboxError::ProjectNotFound { project_id })
    }
}

struct StoredProject {
    id: i64,
    identity: RepositoryIdentity,
    name: String,
    unmerged_chat_count: u32,
}

struct StoredInboxSnapshot {
    projects: Vec<StoredProject>,
    drafts: Vec<DraftSummary>,
    chats: Vec<ChatSummary>,
}

impl StoredInboxSnapshot {
    fn materialize(self, inspector: &dyn RepositoryInspector) -> InboxSnapshot {
        let projects = self
            .projects
            .into_iter()
            .map(|stored| ProjectSummary {
                id: stored.id,
                availability: repository_availability(inspector, &stored.identity),
                name: stored.name,
                unmerged_chat_count: stored.unmerged_chat_count,
            })
            .collect();
        InboxSnapshot {
            projects,
            drafts: self.drafts,
            chats: self.chats,
        }
    }
}

fn load_stored_snapshot(connection: &Connection) -> Result<StoredInboxSnapshot, ProjectInboxError> {
    let mut project_statement = connection
        .prepare(
            "SELECT projects.id, projects.canonical_path, projects.root_device,
                    projects.root_inode, projects.git_dir_path, projects.git_dir_device,
                    projects.git_dir_inode, projects.name,
                    COUNT(chats.id) FILTER (WHERE chats.merge_state = 'unmerged')
             FROM projects
             LEFT JOIN chats ON chats.project_id = projects.id
             GROUP BY projects.id
             ORDER BY projects.created_at_ms ASC, projects.id ASC",
        )
        .map_err(DatabaseError::Query)?;
    let project_rows = project_statement
        .query_map([], |row| {
            let canonical_path: String = row.get(1)?;
            let git_dir_path: String = row.get(4)?;
            Ok(StoredProject {
                id: row.get(0)?,
                identity: RepositoryIdentity {
                    canonical_path: PathBuf::from(canonical_path),
                    root_device: row.get(2)?,
                    root_inode: row.get(3)?,
                    git_dir_path: PathBuf::from(git_dir_path),
                    git_dir_device: row.get(5)?,
                    git_dir_inode: row.get(6)?,
                },
                name: row.get(7)?,
                unmerged_chat_count: row.get(8)?,
            })
        })
        .map_err(DatabaseError::Query)?;
    let projects = project_rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(DatabaseError::Query)?;
    drop(project_statement);

    let mut draft_statement = connection
        .prepare(
            "SELECT project_id, prompt, updated_at_ms
             FROM chat_drafts ORDER BY project_id ASC",
        )
        .map_err(DatabaseError::Query)?;
    let drafts = draft_statement
        .query_map([], |row| {
            Ok(DraftSummary {
                project_id: row.get(0)?,
                prompt: row.get(1)?,
                updated_at_ms: row.get(2)?,
            })
        })
        .map_err(DatabaseError::Query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DatabaseError::Query)?;
    drop(draft_statement);

    let mut chat_statement = connection
        .prepare(
            "SELECT id, project_id, project_name, title, branch_name,
                    pull_request_number, created_at_ms, merge_state
             FROM chats ORDER BY created_at_ms DESC, id ASC",
        )
        .map_err(DatabaseError::Query)?;
    let chats = chat_statement
        .query_map([], |row| {
            let merge_state: String = row.get(7)?;
            Ok(ChatSummary {
                id: row.get(0)?,
                project_id: row.get(1)?,
                project_name: row.get(2)?,
                title: row.get(3)?,
                branch_name: row.get(4)?,
                pull_request_number: row.get(5)?,
                created_at_ms: row.get(6)?,
                merge_state: if merge_state == "merged" {
                    ChatMergeState::Merged
                } else {
                    ChatMergeState::Unmerged
                },
            })
        })
        .map_err(DatabaseError::Query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DatabaseError::Query)?;

    Ok(StoredInboxSnapshot {
        projects,
        drafts,
        chats,
    })
}

fn repository_availability(
    inspector: &dyn RepositoryInspector,
    expected: &RepositoryIdentity,
) -> ProjectAvailability {
    match inspector.inspect(&expected.canonical_path) {
        Ok(actual) if actual == *expected => ProjectAvailability::Available,
        Ok(_) | Err(RepositoryInspectionError::Missing) => ProjectAvailability::Missing,
        Err(RepositoryInspectionError::Inaccessible | RepositoryInspectionError::Git(_)) => {
            ProjectAvailability::Inaccessible
        }
    }
}

fn now_ms() -> Result<i64, ProjectInboxError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProjectInboxError::SystemClock)?
        .as_millis()
        .try_into()
        .map_err(|_| ProjectInboxError::SystemClock)
}

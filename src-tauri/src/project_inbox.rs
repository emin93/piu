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
    prompt_attachments::{
        PromptAttachment, PromptAttachmentError, validate as validate_attachments,
    },
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ChatSetupPhase {
    Pending,
    NotRequired,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ChatSetupFailureKind {
    NotExecutable,
    Launch,
    Exit,
    Signal,
    Interrupted,
    Storage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatSetupSummary {
    pub phase: ChatSetupPhase,
    pub failure: Option<ChatSetupFailureKind>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub attempt: u32,
    pub log: String,
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
    pub attachments: Vec<PromptAttachment>,
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
    pub setup: ChatSetupSummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatSessionReference {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InitialPrompt {
    pub text: String,
    pub attachments: Vec<PromptAttachment>,
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
    #[error("chat {chat_id} does not exist")]
    ChatNotFound { chat_id: String },
    #[error("a chat title cannot be empty")]
    InvalidChatTitle,
    #[error("chat {chat_id} is already bound to another Pi session")]
    ChatSessionAlreadyBound { chat_id: String },
    #[error("chat {chat_id} received an invalid Pi session reference")]
    InvalidChatSessionReference { chat_id: String },
    #[error("project has {count} unmerged chats")]
    ProjectHasUnmergedChats { count: u32 },
    #[error(transparent)]
    Attachment(#[from] PromptAttachmentError),
    #[error("stored prompt attachments are invalid")]
    InvalidAttachmentState,
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

#[derive(Clone, Debug)]
pub(crate) struct ProjectLocation {
    pub id: i64,
    pub name: String,
    pub canonical_path: PathBuf,
    pub git_dir_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FilesystemIdentity {
    pub path: PathBuf,
    pub device: String,
    pub inode: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ChatCreationReservation {
    pub chat_id: String,
    pub project: ProjectLocation,
    pub prompt: String,
    pub attachments: Vec<PromptAttachment>,
    pub title: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub worktree_root: FilesystemIdentity,
    pub worktree_git_dir: Option<FilesystemIdentity>,
    pub base_commit: String,
    pub created_at_ms: i64,
    pub worktree_created: bool,
    pub branch_attached: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ChatWorkspaceOwnership {
    pub chat_id: String,
    pub project_id: i64,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub worktree_root: FilesystemIdentity,
    pub worktree_git_dir: FilesystemIdentity,
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
        attachments: &[PromptAttachment],
    ) -> Result<DraftSummary, ProjectInboxError> {
        let attachments_json = encode_attachments(attachments)?;
        let updated_at_ms = now_ms()?;
        self.with_database(|database| {
            let connection = database.connection_mut();
            let transaction = connection
                .transaction()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            require_project(&transaction, project_id)?;
            if prompt.is_empty() && attachments.is_empty() {
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
                        "INSERT INTO chat_drafts (project_id, prompt, attachments_json, updated_at_ms)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(project_id) DO UPDATE SET
                           prompt = excluded.prompt,
                           attachments_json = excluded.attachments_json,
                           updated_at_ms = excluded.updated_at_ms",
                        params![project_id, prompt, attachments_json, updated_at_ms],
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
                attachments: attachments.to_vec(),
                updated_at_ms,
            })
        })
    }

    pub fn rename_chat(
        &self,
        chat_id: &str,
        title: &str,
    ) -> Result<InboxSnapshot, ProjectInboxError> {
        let title = normalized_chat_title(title);
        if title.is_empty() {
            return Err(ProjectInboxError::InvalidChatTitle);
        }
        self.with_database(|database| {
            let changed = database
                .connection_mut()
                .execute(
                    "UPDATE chats SET title = ?1 WHERE id = ?2",
                    params![title, chat_id],
                )
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            if changed == 0 {
                return Err(ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                });
            }
            Ok(())
        })?;
        self.snapshot()
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
                    "SELECT
                       (SELECT COUNT(*) FROM chats
                        WHERE project_id = ?1 AND merge_state = 'unmerged') +
                       (SELECT COUNT(*) FROM chat_workspace_creations WHERE project_id = ?1)",
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

    pub(crate) fn project_location(
        &self,
        project_id: i64,
    ) -> Result<ProjectLocation, ProjectInboxError> {
        let (project, expected) = self.with_database(|database| {
            database
                .connection()
                .query_row(
                    "SELECT id, name, canonical_path, root_device, root_inode,
                            git_dir_path, git_dir_device, git_dir_inode
                     FROM projects WHERE id = ?1",
                    [project_id],
                    |row| {
                        let canonical_path: String = row.get(2)?;
                        let git_dir_path: String = row.get(5)?;
                        let expected = RepositoryIdentity {
                            canonical_path: PathBuf::from(&canonical_path),
                            root_device: row.get(3)?,
                            root_inode: row.get(4)?,
                            git_dir_path: PathBuf::from(&git_dir_path),
                            git_dir_device: row.get(6)?,
                            git_dir_inode: row.get(7)?,
                        };
                        Ok((
                            ProjectLocation {
                                id: row.get(0)?,
                                name: row.get(1)?,
                                canonical_path: PathBuf::from(canonical_path),
                                git_dir_path: PathBuf::from(git_dir_path),
                            },
                            expected,
                        ))
                    },
                )
                .optional()
                .map_err(DatabaseError::Query)?
                .ok_or(ProjectInboxError::ProjectNotFound { project_id })
        })?;
        let actual = self
            .repository_inspector
            .inspect(&expected.canonical_path)
            .map_err(map_admission_error)?;
        if actual != expected {
            return Err(ProjectInboxError::InvalidRepository);
        }
        Ok(project)
    }

    pub(crate) fn allocate_chat_id(&self) -> Result<String, ProjectInboxError> {
        self.with_database(|database| {
            database
                .connection()
                .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)
        })
    }

    pub(crate) fn reserve_chat_creation(
        &self,
        reservation: &ChatCreationReservation,
    ) -> Result<(), ProjectInboxError> {
        let attachments_json = encode_attachments(&reservation.attachments)?;
        self.with_database(|database| {
            database
                .connection_mut()
                .execute(
                    "INSERT INTO chat_workspace_creations (
                       chat_id, project_id, project_name, prompt, attachments_json, title, branch_name,
                       worktree_path, worktree_root_path, worktree_root_device, worktree_root_inode,
                       base_commit, created_at_ms, worktree_created, branch_attached,
                       worktree_git_dir_path, worktree_git_dir_device, worktree_git_dir_inode
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, 0,
                               NULL, NULL, NULL)",
                    params![
                        reservation.chat_id,
                        reservation.project.id,
                        reservation.project.name,
                        reservation.prompt,
                        attachments_json,
                        reservation.title,
                        reservation.branch_name,
                        reservation.worktree_path.to_string_lossy(),
                        reservation.worktree_root.path.to_string_lossy(),
                        reservation.worktree_root.device,
                        reservation.worktree_root.inode,
                        reservation.base_commit,
                        reservation.created_at_ms,
                    ],
                )
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            Ok(())
        })
    }

    pub(crate) fn mark_creation_worktree_created(
        &self,
        chat_id: &str,
        git_dir: &FilesystemIdentity,
    ) -> Result<(), ProjectInboxError> {
        self.with_database(|database| {
            let changed = database
                .connection_mut()
                .execute(
                    "UPDATE chat_workspace_creations
                     SET worktree_created = 1, worktree_git_dir_path = ?2,
                         worktree_git_dir_device = ?3, worktree_git_dir_inode = ?4
                     WHERE chat_id = ?1 AND worktree_created = 0",
                    params![
                        chat_id,
                        git_dir.path.to_string_lossy(),
                        git_dir.device,
                        git_dir.inode,
                    ],
                )
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            if changed == 1 {
                Ok(())
            } else {
                Err(ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                })
            }
        })
    }

    pub(crate) fn mark_creation_branch_attached(
        &self,
        chat_id: &str,
    ) -> Result<(), ProjectInboxError> {
        self.update_creation_flag(chat_id, "branch_attached")
    }

    fn update_creation_flag(&self, chat_id: &str, column: &str) -> Result<(), ProjectInboxError> {
        debug_assert_eq!(column, "branch_attached");
        self.with_database(|database| {
            let changed = database
                .connection_mut()
                .execute(
                    &format!("UPDATE chat_workspace_creations SET {column} = 1 WHERE chat_id = ?1"),
                    [chat_id],
                )
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            if changed == 1 {
                Ok(())
            } else {
                Err(ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                })
            }
        })
    }

    pub(crate) fn commit_chat_creation(
        &self,
        reservation: &ChatCreationReservation,
    ) -> Result<(), ProjectInboxError> {
        let worktree_git_dir = reservation.worktree_git_dir.as_ref().ok_or_else(|| {
            ProjectInboxError::ChatNotFound {
                chat_id: reservation.chat_id.clone(),
            }
        })?;
        let attachments_json = encode_attachments(&reservation.attachments)?;
        self.with_database(|database| {
            let transaction = database
                .connection_mut()
                .transaction()
                .map_err(DatabaseError::Query)?;
            transaction
                .execute(
                    "INSERT INTO chats (
                       id, project_id, project_name, title, branch_name, worktree_path,
                       worktree_root_path, worktree_root_device, worktree_root_inode,
                       worktree_git_dir_path, worktree_git_dir_device, worktree_git_dir_inode,
                       base_commit, pull_request_number, created_at_ms, merge_state,
                       setup_phase, setup_failure, setup_exit_code, setup_signal,
                       setup_attempt, setup_log, initial_attachments_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                               ?13, NULL, ?14, 'unmerged',
                               'pending', NULL, NULL, NULL, 0, '', ?15)",
                    params![
                        reservation.chat_id,
                        reservation.project.id,
                        reservation.project.name,
                        reservation.title,
                        reservation.branch_name,
                        reservation.worktree_path.to_string_lossy(),
                        reservation.worktree_root.path.to_string_lossy(),
                        reservation.worktree_root.device,
                        reservation.worktree_root.inode,
                        worktree_git_dir.path.to_string_lossy(),
                        worktree_git_dir.device,
                        worktree_git_dir.inode,
                        reservation.base_commit,
                        reservation.created_at_ms,
                        attachments_json,
                    ],
                )
                .map_err(DatabaseError::Query)?;
            transaction
                .execute(
                    "INSERT INTO chat_messages (chat_id, sequence, role, content, created_at_ms)
                     VALUES (?1, 1, 'user', ?2, ?3)",
                    params![
                        reservation.chat_id,
                        reservation.prompt,
                        reservation.created_at_ms
                    ],
                )
                .map_err(DatabaseError::Query)?;
            transaction
                .execute(
                    "DELETE FROM chat_drafts WHERE project_id = ?1",
                    [reservation.project.id],
                )
                .map_err(DatabaseError::Query)?;
            let removed = transaction
                .execute(
                    "DELETE FROM chat_workspace_creations WHERE chat_id = ?1",
                    [&reservation.chat_id],
                )
                .map_err(DatabaseError::Query)?;
            if removed != 1 {
                return Err(ProjectInboxError::ChatNotFound {
                    chat_id: reservation.chat_id.clone(),
                });
            }
            transaction.commit().map_err(DatabaseError::Query)?;
            Ok(())
        })
    }

    pub(crate) fn pending_chat_creations(
        &self,
    ) -> Result<Vec<ChatCreationReservation>, ProjectInboxError> {
        self.with_database(|database| {
            let mut statement = database
                .connection()
                .prepare(
                    "SELECT journal.chat_id, journal.project_id, journal.project_name,
                            projects.canonical_path, projects.git_dir_path,
                            journal.prompt, journal.title,
                            journal.branch_name, journal.worktree_path,
                            journal.worktree_root_path, journal.worktree_root_device,
                            journal.worktree_root_inode,
                            journal.base_commit, journal.created_at_ms, journal.worktree_created,
                            journal.branch_attached, journal.worktree_git_dir_path,
                            journal.worktree_git_dir_device, journal.worktree_git_dir_inode,
                            journal.attachments_json
                     FROM chat_workspace_creations AS journal
                     JOIN projects ON projects.id = journal.project_id
                     ORDER BY journal.created_at_ms ASC, journal.chat_id ASC",
                )
                .map_err(DatabaseError::Query)?;
            statement
                .query_map([], |row| {
                    let canonical_path: String = row.get(3)?;
                    let project_git_dir_path: String = row.get(4)?;
                    let worktree_path: String = row.get(8)?;
                    let worktree_root_path: String = row.get(9)?;
                    let git_dir_path: Option<String> = row.get(16)?;
                    let git_dir_device: Option<String> = row.get(17)?;
                    let git_dir_inode: Option<String> = row.get(18)?;
                    let attachments_json: String = row.get(19)?;
                    let worktree_git_dir = match (git_dir_path, git_dir_device, git_dir_inode) {
                        (Some(path), Some(device), Some(inode)) => Some(FilesystemIdentity {
                            path: PathBuf::from(path),
                            device,
                            inode,
                        }),
                        (None, None, None) => None,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    };
                    Ok(ChatCreationReservation {
                        chat_id: row.get(0)?,
                        project: ProjectLocation {
                            id: row.get(1)?,
                            name: row.get(2)?,
                            canonical_path: PathBuf::from(canonical_path),
                            git_dir_path: PathBuf::from(project_git_dir_path),
                        },
                        prompt: row.get(5)?,
                        attachments: decode_attachments(&attachments_json)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        title: row.get(6)?,
                        branch_name: row.get(7)?,
                        worktree_path: PathBuf::from(&worktree_path),
                        worktree_root: FilesystemIdentity {
                            path: PathBuf::from(worktree_root_path),
                            device: row.get(10)?,
                            inode: row.get(11)?,
                        },
                        worktree_git_dir,
                        base_commit: row.get(12)?,
                        created_at_ms: row.get(13)?,
                        worktree_created: row.get(14)?,
                        branch_attached: row.get(15)?,
                    })
                })
                .map_err(DatabaseError::Query)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)
        })
    }

    pub(crate) fn discard_chat_creation(&self, chat_id: &str) -> Result<(), ProjectInboxError> {
        self.with_database(|database| {
            database
                .connection_mut()
                .execute(
                    "DELETE FROM chat_workspace_creations WHERE chat_id = ?1",
                    [chat_id],
                )
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            Ok(())
        })
    }

    pub(crate) fn chat_workspace_ownership(
        &self,
        chat_id: &str,
    ) -> Result<ChatWorkspaceOwnership, ProjectInboxError> {
        self.with_database(|database| {
            database
                .connection()
                .query_row(
                    "SELECT id, project_id, branch_name, worktree_path,
                            worktree_root_path, worktree_root_device, worktree_root_inode,
                            worktree_git_dir_path, worktree_git_dir_device, worktree_git_dir_inode
                     FROM chats WHERE id = ?1",
                    [chat_id],
                    |row| {
                        let worktree_path: String = row.get(3)?;
                        let worktree_root_path: String = row.get(4)?;
                        let worktree_git_dir_path: String = row.get(7)?;
                        Ok(ChatWorkspaceOwnership {
                            chat_id: row.get(0)?,
                            project_id: row.get(1)?,
                            branch_name: row.get(2)?,
                            worktree_path: PathBuf::from(worktree_path),
                            worktree_root: FilesystemIdentity {
                                path: PathBuf::from(worktree_root_path),
                                device: row.get(5)?,
                                inode: row.get(6)?,
                            },
                            worktree_git_dir: FilesystemIdentity {
                                path: PathBuf::from(worktree_git_dir_path),
                                device: row.get(8)?,
                                inode: row.get(9)?,
                            },
                        })
                    },
                )
                .optional()
                .map_err(DatabaseError::Query)?
                .ok_or_else(|| ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                })
        })
    }

    pub fn chat_session(
        &self,
        chat_id: &str,
    ) -> Result<Option<ChatSessionReference>, ProjectInboxError> {
        self.with_database(|database| {
            let stored = database
                .connection()
                .query_row(
                    "SELECT pi_session_id, pi_session_path FROM chats WHERE id = ?1",
                    [chat_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .optional()
                .map_err(DatabaseError::Query)?
                .ok_or_else(|| ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                })?;
            match stored {
                (None, None) => Ok(None),
                (Some(id), Some(path)) => Ok(Some(ChatSessionReference {
                    id,
                    path: PathBuf::from(path),
                })),
                _ => Err(ProjectInboxError::Database(DatabaseError::Query(
                    rusqlite::Error::InvalidQuery,
                ))),
            }
        })
    }

    pub fn bind_chat_session(
        &self,
        chat_id: &str,
        session_id: &str,
        session_path: &Path,
    ) -> Result<ChatSessionReference, ProjectInboxError> {
        if session_id.is_empty() || !session_path.is_absolute() {
            return Err(ProjectInboxError::InvalidChatSessionReference {
                chat_id: chat_id.to_owned(),
            });
        }
        let session_path = session_path.to_str().ok_or_else(|| {
            ProjectInboxError::InvalidChatSessionReference {
                chat_id: chat_id.to_owned(),
            }
        })?;
        self.with_database(|database| {
            let connection = database.connection_mut();
            let transaction = connection.transaction().map_err(DatabaseError::Query)?;
            let stored = transaction
                .query_row(
                    "SELECT pi_session_id, pi_session_path FROM chats WHERE id = ?1",
                    [chat_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .optional()
                .map_err(DatabaseError::Query)?
                .ok_or_else(|| ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                })?;
            match stored {
                (Some(stored_id), Some(stored_path))
                    if stored_id == session_id && stored_path == session_path => {}
                (Some(_), Some(_)) => {
                    return Err(ProjectInboxError::ChatSessionAlreadyBound {
                        chat_id: chat_id.to_owned(),
                    });
                }
                (None, None) => {
                    transaction
                        .execute(
                            "UPDATE chats SET pi_session_id = ?2, pi_session_path = ?3
                             WHERE id = ?1 AND pi_session_id IS NULL AND pi_session_path IS NULL",
                            params![chat_id, session_id, session_path],
                        )
                        .map_err(DatabaseError::Query)?;
                }
                _ => {
                    return Err(ProjectInboxError::Database(DatabaseError::Query(
                        rusqlite::Error::InvalidQuery,
                    )));
                }
            }
            transaction.commit().map_err(DatabaseError::Query)?;
            Ok(ChatSessionReference {
                id: session_id.to_owned(),
                path: PathBuf::from(session_path),
            })
        })
    }

    #[cfg(test)]
    pub(crate) fn first_user_message(&self, chat_id: &str) -> Result<String, ProjectInboxError> {
        self.initial_prompt(chat_id).map(|prompt| prompt.text)
    }

    pub(crate) fn initial_prompt(&self, chat_id: &str) -> Result<InitialPrompt, ProjectInboxError> {
        self.with_database(|database| {
            database
                .connection()
                .query_row(
                    "SELECT messages.content, chats.initial_attachments_json
                     FROM chat_messages AS messages
                     JOIN chats ON chats.id = messages.chat_id
                     WHERE messages.chat_id = ?1 AND messages.sequence = 1
                       AND messages.role = 'user'",
                    [chat_id],
                    |row| {
                        let attachments_json: String = row.get(1)?;
                        Ok(InitialPrompt {
                            text: row.get(0)?,
                            attachments: decode_attachments(&attachments_json)
                                .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        })
                    },
                )
                .optional()
                .map_err(DatabaseError::Query)?
                .ok_or_else(|| ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                })
        })
    }

    pub(crate) fn begin_setup(&self, chat_id: &str) -> Result<ChatSetupSummary, ProjectInboxError> {
        self.with_database(|database| {
            let connection = database.connection_mut();
            let changed = connection
                .execute(
                    "UPDATE chats SET setup_phase = 'running', setup_failure = NULL,
                         setup_exit_code = NULL, setup_signal = NULL,
                         setup_attempt = setup_attempt + 1, setup_log = ''
                     WHERE id = ?1 AND setup_phase != 'running'",
                    [chat_id],
                )
                .map_err(DatabaseError::Query)?;
            if changed != 1 {
                return Err(ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                });
            }
            load_chat_setup(connection, chat_id)
        })
    }

    pub(crate) fn mark_setup_not_required(
        &self,
        chat_id: &str,
    ) -> Result<ChatSetupSummary, ProjectInboxError> {
        self.finish_setup(chat_id, ChatSetupPhase::NotRequired, None, None, None)
    }

    pub(crate) fn chat_setup(&self, chat_id: &str) -> Result<ChatSetupSummary, ProjectInboxError> {
        self.with_database(|database| load_chat_setup(database.connection(), chat_id))
    }

    pub(crate) fn append_setup_log(
        &self,
        chat_id: &str,
        attempt: u32,
        chunk: &str,
    ) -> Result<ChatSetupSummary, ProjectInboxError> {
        self.with_database(|database| {
            let connection = database.connection_mut();
            let changed = connection
                .execute(
                    "UPDATE chats SET setup_log = setup_log || ?3
                     WHERE id = ?1 AND setup_phase = 'running' AND setup_attempt = ?2",
                    params![chat_id, attempt, chunk],
                )
                .map_err(DatabaseError::Query)?;
            if changed != 1 {
                return Err(ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                });
            }
            load_chat_setup(connection, chat_id)
        })
    }

    pub(crate) fn finish_setup(
        &self,
        chat_id: &str,
        phase: ChatSetupPhase,
        failure: Option<ChatSetupFailureKind>,
        exit_code: Option<i32>,
        signal: Option<i32>,
    ) -> Result<ChatSetupSummary, ProjectInboxError> {
        let phase = setup_phase_to_storage(phase);
        let failure = failure.map(setup_failure_to_storage);
        self.with_database(|database| {
            let connection = database.connection_mut();
            let changed = connection
                .execute(
                    "UPDATE chats SET setup_phase = ?2, setup_failure = ?3,
                         setup_exit_code = ?4, setup_signal = ?5
                     WHERE id = ?1",
                    params![chat_id, phase, failure, exit_code, signal],
                )
                .map_err(DatabaseError::Query)?;
            if changed != 1 {
                return Err(ProjectInboxError::ChatNotFound {
                    chat_id: chat_id.to_owned(),
                });
            }
            load_chat_setup(connection, chat_id)
        })
    }

    pub(crate) fn interrupt_incomplete_setups(&self) -> Result<(), ProjectInboxError> {
        self.with_database(|database| {
            database
                .connection_mut()
                .execute(
                    "UPDATE chats SET setup_phase = 'failed', setup_failure = 'interrupted',
                         setup_exit_code = NULL, setup_signal = NULL
                     WHERE setup_phase IN ('pending', 'running')",
                    [],
                )
                .map_err(DatabaseError::Query)
                .map_err(ProjectInboxError::Database)?;
            Ok(())
        })
    }

    pub(crate) fn with_database<T>(
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

pub(crate) fn normalized_chat_title(input: &str) -> String {
    let collapsed = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = collapsed.chars().take(72).collect::<String>();
    if collapsed.chars().count() > 72 {
        title.push('…');
    }
    title
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
            "SELECT project_id, prompt, attachments_json, updated_at_ms
             FROM chat_drafts ORDER BY project_id ASC",
        )
        .map_err(DatabaseError::Query)?;
    let drafts = draft_statement
        .query_map([], |row| {
            let attachments_json: String = row.get(2)?;
            Ok(DraftSummary {
                project_id: row.get(0)?,
                prompt: row.get(1)?,
                attachments: decode_attachments(&attachments_json)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                updated_at_ms: row.get(3)?,
            })
        })
        .map_err(DatabaseError::Query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DatabaseError::Query)?;
    drop(draft_statement);

    let mut chat_statement = connection
        .prepare(
            "SELECT id, project_id, project_name, title, branch_name,
                    pull_request_number, created_at_ms, merge_state, setup_phase,
                    setup_failure, setup_exit_code, setup_signal, setup_attempt, setup_log
             FROM chats ORDER BY created_at_ms DESC, id ASC",
        )
        .map_err(DatabaseError::Query)?;
    let chats = chat_statement
        .query_map([], |row| {
            let merge_state: String = row.get(7)?;
            let setup_phase: String = row.get(8)?;
            let setup_failure: Option<String> = row.get(9)?;
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
                setup: ChatSetupSummary {
                    phase: setup_phase_from_storage(&setup_phase),
                    failure: setup_failure.as_deref().map(setup_failure_from_storage),
                    exit_code: row.get(10)?,
                    signal: row.get(11)?,
                    attempt: row.get(12)?,
                    log: row.get(13)?,
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

fn load_chat_setup(
    connection: &Connection,
    chat_id: &str,
) -> Result<ChatSetupSummary, ProjectInboxError> {
    connection
        .query_row(
            "SELECT setup_phase, setup_failure, setup_exit_code, setup_signal,
                    setup_attempt, setup_log
             FROM chats WHERE id = ?1",
            [chat_id],
            |row| {
                let phase: String = row.get(0)?;
                let failure: Option<String> = row.get(1)?;
                Ok(ChatSetupSummary {
                    phase: setup_phase_from_storage(&phase),
                    failure: failure.as_deref().map(setup_failure_from_storage),
                    exit_code: row.get(2)?,
                    signal: row.get(3)?,
                    attempt: row.get(4)?,
                    log: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(DatabaseError::Query)?
        .ok_or_else(|| ProjectInboxError::ChatNotFound {
            chat_id: chat_id.to_owned(),
        })
}

fn setup_phase_to_storage(phase: ChatSetupPhase) -> &'static str {
    match phase {
        ChatSetupPhase::Pending => "pending",
        ChatSetupPhase::NotRequired => "not_required",
        ChatSetupPhase::Running => "running",
        ChatSetupPhase::Succeeded => "succeeded",
        ChatSetupPhase::Failed => "failed",
        ChatSetupPhase::Cancelled => "cancelled",
    }
}

fn setup_phase_from_storage(phase: &str) -> ChatSetupPhase {
    match phase {
        "pending" => ChatSetupPhase::Pending,
        "not_required" => ChatSetupPhase::NotRequired,
        "running" => ChatSetupPhase::Running,
        "succeeded" => ChatSetupPhase::Succeeded,
        "cancelled" => ChatSetupPhase::Cancelled,
        _ => ChatSetupPhase::Failed,
    }
}

fn setup_failure_to_storage(failure: ChatSetupFailureKind) -> &'static str {
    match failure {
        ChatSetupFailureKind::NotExecutable => "not_executable",
        ChatSetupFailureKind::Launch => "launch",
        ChatSetupFailureKind::Exit => "exit",
        ChatSetupFailureKind::Signal => "signal",
        ChatSetupFailureKind::Interrupted => "interrupted",
        ChatSetupFailureKind::Storage => "storage",
    }
}

fn setup_failure_from_storage(failure: &str) -> ChatSetupFailureKind {
    match failure {
        "not_executable" => ChatSetupFailureKind::NotExecutable,
        "launch" => ChatSetupFailureKind::Launch,
        "exit" => ChatSetupFailureKind::Exit,
        "signal" => ChatSetupFailureKind::Signal,
        "interrupted" => ChatSetupFailureKind::Interrupted,
        _ => ChatSetupFailureKind::Storage,
    }
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

fn encode_attachments(attachments: &[PromptAttachment]) -> Result<String, ProjectInboxError> {
    validate_attachments(attachments)?;
    serde_json::to_string(attachments).map_err(|_| ProjectInboxError::InvalidAttachmentState)
}

fn decode_attachments(value: &str) -> Result<Vec<PromptAttachment>, ProjectInboxError> {
    let attachments = serde_json::from_str::<Vec<PromptAttachment>>(value)
        .map_err(|_| ProjectInboxError::InvalidAttachmentState)?;
    validate_attachments(&attachments)?;
    Ok(attachments)
}

fn now_ms() -> Result<i64, ProjectInboxError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProjectInboxError::SystemClock)?
        .as_millis()
        .try_into()
        .map_err(|_| ProjectInboxError::SystemClock)
}

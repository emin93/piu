use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params};
use thiserror::Error;

const CURRENT_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS projects (
        id INTEGER PRIMARY KEY,
        canonical_path TEXT NOT NULL UNIQUE,
        root_device TEXT NOT NULL,
        root_inode TEXT NOT NULL,
        git_dir_path TEXT NOT NULL,
        git_dir_device TEXT NOT NULL,
        git_dir_inode TEXT NOT NULL,
        name TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS runtime_model_selection (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        provider_id TEXT NOT NULL CHECK (length(provider_id) > 0),
        model_id TEXT NOT NULL CHECK (length(model_id) > 0)
    );

    CREATE TABLE IF NOT EXISTS model_route_efforts (
        provider_id TEXT NOT NULL CHECK (length(provider_id) > 0),
        model_id TEXT NOT NULL CHECK (length(model_id) > 0),
        effort TEXT NOT NULL CHECK (length(effort) > 0),
        PRIMARY KEY (provider_id, model_id)
    );

    CREATE TABLE IF NOT EXISTS global_resource_enable_overrides (
        resource_kind TEXT NOT NULL CHECK (
            resource_kind IN ('model_route', 'skill', 'extension', 'package')
        ),
        provider_id TEXT NOT NULL,
        resource_id TEXT NOT NULL CHECK (length(resource_id) > 0),
        enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
        PRIMARY KEY (resource_kind, provider_id, resource_id),
        CHECK (
            (resource_kind = 'model_route' AND length(provider_id) > 0)
            OR (resource_kind != 'model_route' AND provider_id = '')
        )
    );

    CREATE TABLE IF NOT EXISTS project_resource_enable_overrides (
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        resource_kind TEXT NOT NULL CHECK (
            resource_kind IN ('model_route', 'skill', 'extension', 'package')
        ),
        provider_id TEXT NOT NULL,
        resource_id TEXT NOT NULL CHECK (length(resource_id) > 0),
        enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
        PRIMARY KEY (project_id, resource_kind, provider_id, resource_id),
        CHECK (
            (resource_kind = 'model_route' AND length(provider_id) > 0)
            OR (resource_kind != 'model_route' AND provider_id = '')
        )
    );

    CREATE TABLE IF NOT EXISTS chat_drafts (
        project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
        prompt TEXT NOT NULL,
        attachments_json TEXT NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS chats (
        id TEXT PRIMARY KEY,
        project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
        project_name TEXT NOT NULL,
        title TEXT NOT NULL,
        branch_name TEXT NOT NULL,
        worktree_path TEXT NOT NULL UNIQUE,
        worktree_root_path TEXT NOT NULL UNIQUE,
        worktree_root_device TEXT NOT NULL,
        worktree_root_inode TEXT NOT NULL,
        worktree_git_dir_path TEXT NOT NULL UNIQUE,
        worktree_git_dir_device TEXT NOT NULL,
        worktree_git_dir_inode TEXT NOT NULL,
        base_commit TEXT NOT NULL,
        pull_request_number INTEGER,
        created_at_ms INTEGER NOT NULL,
        merge_state TEXT NOT NULL CHECK (merge_state IN ('unmerged', 'merged')),
        setup_phase TEXT NOT NULL CHECK (
            setup_phase IN ('pending', 'not_required', 'running', 'succeeded', 'failed', 'cancelled')
        ),
        setup_failure TEXT CHECK (
            setup_failure IS NULL OR setup_failure IN (
                'not_executable', 'launch', 'exit', 'signal', 'interrupted', 'storage'
            )
        ),
        setup_exit_code INTEGER,
        setup_signal INTEGER,
        setup_attempt INTEGER NOT NULL,
        setup_log TEXT NOT NULL,
        initial_attachments_json TEXT NOT NULL,
        initial_model_provider TEXT,
        initial_model_id TEXT,
        initial_reasoning_effort TEXT,
        pi_session_id TEXT UNIQUE,
        pi_session_path TEXT UNIQUE,
        CHECK (
            (initial_model_provider IS NULL AND initial_model_id IS NULL
                AND initial_reasoning_effort IS NULL)
            OR (initial_model_provider IS NOT NULL AND initial_model_id IS NOT NULL
                AND length(initial_model_provider) > 0 AND length(initial_model_id) > 0
                AND (initial_reasoning_effort IS NULL OR length(initial_reasoning_effort) > 0))
        ),
        CHECK (
            (pi_session_id IS NULL AND pi_session_path IS NULL)
            OR (pi_session_id IS NOT NULL AND pi_session_path IS NOT NULL)
        )
    );

    CREATE TABLE IF NOT EXISTS chat_messages (
        chat_id TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
        sequence INTEGER NOT NULL,
        role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
        content TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        PRIMARY KEY (chat_id, sequence)
    );

    CREATE TABLE IF NOT EXISTS chat_workspace_creations (
        chat_id TEXT PRIMARY KEY,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
        project_name TEXT NOT NULL,
        prompt TEXT NOT NULL,
        attachments_json TEXT NOT NULL,
        title TEXT NOT NULL,
        branch_name TEXT NOT NULL UNIQUE,
        worktree_path TEXT NOT NULL UNIQUE,
        worktree_root_path TEXT NOT NULL UNIQUE,
        worktree_root_device TEXT NOT NULL,
        worktree_root_inode TEXT NOT NULL,
        base_commit TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        worktree_created INTEGER NOT NULL CHECK (worktree_created IN (0, 1)),
        branch_attached INTEGER NOT NULL CHECK (branch_attached IN (0, 1)),
        worktree_git_dir_path TEXT,
        worktree_git_dir_device TEXT,
        worktree_git_dir_inode TEXT,
        initial_model_provider TEXT,
        initial_model_id TEXT,
        initial_reasoning_effort TEXT,
        CHECK (
            (initial_model_provider IS NULL AND initial_model_id IS NULL
                AND initial_reasoning_effort IS NULL)
            OR (initial_model_provider IS NOT NULL AND initial_model_id IS NOT NULL
                AND length(initial_model_provider) > 0 AND length(initial_model_id) > 0
                AND (initial_reasoning_effort IS NULL OR length(initial_reasoning_effort) > 0))
        ),
        CHECK (
            (worktree_created = 0 AND worktree_git_dir_path IS NULL
                AND worktree_git_dir_device IS NULL AND worktree_git_dir_inode IS NULL)
            OR
            (worktree_created = 1 AND worktree_git_dir_path IS NOT NULL
                AND worktree_git_dir_device IS NOT NULL AND worktree_git_dir_inode IS NOT NULL)
        )
    );

    CREATE INDEX IF NOT EXISTS chats_created_at ON chats(created_at_ms DESC, id ASC);
    CREATE INDEX IF NOT EXISTS chats_project_id ON chats(project_id);
    CREATE INDEX IF NOT EXISTS chat_messages_chat_id ON chat_messages(chat_id, sequence);
    CREATE INDEX IF NOT EXISTS chat_workspace_creations_project_id
        ON chat_workspace_creations(project_id);
"#;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("could not open application database: {0}")]
    Open(#[source] rusqlite::Error),
    #[error("could not query application database: {0}")]
    Query(#[source] rusqlite::Error),
    #[error("could not initialize application database: {0}")]
    Initialize(#[source] rusqlite::Error),
}

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        let mut connection = Connection::open(path).map_err(DatabaseError::Open)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(DatabaseError::Query)?;
        initialize_current_schema(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn has_table(&self, table_name: &str) -> rusqlite::Result<bool> {
        self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
            params![table_name],
            |row| row.get(0),
        )
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

fn initialize_current_schema(connection: &mut Connection) -> Result<(), DatabaseError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DatabaseError::Initialize)?;
    transaction
        .execute_batch(CURRENT_SCHEMA)
        .map_err(DatabaseError::Initialize)?;
    transaction.commit().map_err(DatabaseError::Initialize)
}

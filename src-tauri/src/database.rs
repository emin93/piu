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

    CREATE TABLE IF NOT EXISTS chat_drafts (
        project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
        prompt TEXT NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS chats (
        id TEXT PRIMARY KEY,
        project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
        project_name TEXT NOT NULL,
        title TEXT NOT NULL,
        branch_name TEXT NOT NULL,
        pull_request_number INTEGER,
        created_at_ms INTEGER NOT NULL,
        merge_state TEXT NOT NULL CHECK (merge_state IN ('unmerged', 'merged'))
    );

    CREATE INDEX IF NOT EXISTS chats_created_at ON chats(created_at_ms DESC, id ASC);
    CREATE INDEX IF NOT EXISTS chats_project_id ON chats(project_id);
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

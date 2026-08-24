use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params};
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

const MIGRATIONS: &[Migration] = &[
    Migration::new(
        1,
        "create application metadata",
        r#"
    CREATE TABLE application_metadata (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        created_at TEXT NOT NULL
    );

    INSERT INTO application_metadata (id, created_at)
    VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
"#,
    ),
    Migration::new(
        2,
        "create project inbox",
        r#"
    CREATE TABLE projects (
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

    CREATE TABLE chat_drafts (
        project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
        prompt TEXT NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );

    CREATE TABLE chats (
        id TEXT PRIMARY KEY,
        project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
        project_name TEXT NOT NULL,
        title TEXT NOT NULL,
        branch_name TEXT NOT NULL,
        pull_request_number INTEGER,
        created_at_ms INTEGER NOT NULL,
        merge_state TEXT NOT NULL CHECK (merge_state IN ('unmerged', 'merged'))
    );

    CREATE INDEX chats_created_at ON chats(created_at_ms DESC, id ASC);
    CREATE INDEX chats_project_id ON chats(project_id);
"#,
    ),
];

#[derive(Clone, Copy, Debug)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

impl Migration {
    pub const fn new(version: u32, name: &'static str, sql: &'static str) -> Self {
        Self { version, name, sql }
    }
}

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("could not open application database: {0}")]
    Open(#[source] rusqlite::Error),
    #[error("could not query application database: {0}")]
    Query(#[source] rusqlite::Error),
    #[error("database migration {version} ({name}) failed: {source}")]
    Migration {
        version: u32,
        name: &'static str,
        #[source]
        source: rusqlite::Error,
    },
}

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        Self::open_with_migrations(path, MIGRATIONS)
    }

    pub fn open_with_migrations(
        path: &Path,
        migrations: &[Migration],
    ) -> Result<Self, DatabaseError> {
        let mut connection = Connection::open(path).map_err(DatabaseError::Open)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(DatabaseError::Query)?;
        migrate(&mut connection, migrations)?;
        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> rusqlite::Result<u32> {
        self.connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
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

fn migrate(connection: &mut Connection, migrations: &[Migration]) -> Result<(), DatabaseError> {
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| DatabaseError::Migration {
            version: 0,
            name: "read current version",
            source,
        })?;
    let pending = migrations
        .iter()
        .filter(|migration| migration.version > version)
        .collect::<Vec<_>>();

    if pending.is_empty() {
        return Ok(());
    }

    let first = pending[0];
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|source| DatabaseError::Migration {
            version: first.version,
            name: first.name,
            source,
        })?;
    for migration in pending {
        transaction
            .execute_batch(migration.sql)
            .and_then(|_| transaction.pragma_update(None, "user_version", migration.version))
            .map_err(|source| DatabaseError::Migration {
                version: migration.version,
                name: migration.name,
                source,
            })?;
    }
    transaction
        .commit()
        .map_err(|source| DatabaseError::Migration {
            version: first.version,
            name: "commit",
            source,
        })
}

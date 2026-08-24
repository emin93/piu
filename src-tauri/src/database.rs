use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params};
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

const MIGRATIONS: &[Migration] = &[Migration::new(
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
)];

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

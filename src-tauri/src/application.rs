use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use thiserror::Error;

use crate::database::{Database, DatabaseError};

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("could not prepare application data at {path}: {source}")]
    AppData {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("application database lock is poisoned")]
    DatabaseLock,
}

pub struct ApplicationCore {
    database_path: PathBuf,
    database: Mutex<Option<Database>>,
}

impl ApplicationCore {
    pub fn open(database_path: &Path) -> Result<Self, ApplicationError> {
        let core = Self::deferred(database_path.to_path_buf());
        core.schema_version()?;
        Ok(core)
    }

    pub fn deferred(database_path: PathBuf) -> Self {
        Self {
            database_path,
            database: Mutex::new(None),
        }
    }

    pub fn schema_version(&self) -> Result<u32, ApplicationError> {
        let mut database = self
            .database
            .lock()
            .map_err(PoisonError::into_inner)
            .map_err(|_| ApplicationError::DatabaseLock)?;
        if database.is_none() {
            if let Some(parent) = self.database_path.parent() {
                fs::create_dir_all(parent).map_err(|source| ApplicationError::AppData {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            *database = Some(Database::open(&self.database_path)?);
        }
        database
            .as_ref()
            .expect("database is initialized before use")
            .schema_version()
            .map_err(DatabaseError::Query)
            .map_err(ApplicationError::Database)
    }
}

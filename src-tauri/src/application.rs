use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::project_inbox::{ProjectInbox, ProjectInboxError};

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    ProjectInbox(#[from] ProjectInboxError),
}

pub struct ApplicationCore {
    project_inbox: ProjectInbox,
}

impl ApplicationCore {
    pub fn open(database_path: &Path) -> Result<Self, ApplicationError> {
        let core = Self::deferred(database_path.to_path_buf());
        core.schema_version()?;
        Ok(core)
    }

    pub fn deferred(database_path: PathBuf) -> Self {
        Self {
            project_inbox: ProjectInbox::deferred(database_path),
        }
    }

    pub fn schema_version(&self) -> Result<u32, ApplicationError> {
        self.project_inbox
            .schema_version()
            .map_err(ApplicationError::ProjectInbox)
    }

    pub fn project_inbox(&self) -> &ProjectInbox {
        &self.project_inbox
    }
}

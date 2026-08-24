use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use thiserror::Error;

use crate::{
    git_process::GitProcess,
    project_inbox::{ProjectInbox, ProjectInboxError},
};

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    ProjectInbox(#[from] ProjectInboxError),
}

pub struct ApplicationCore {
    project_inbox: Arc<ProjectInbox>,
}

impl ApplicationCore {
    pub fn open(database_path: &Path, git: GitProcess) -> Result<Self, ApplicationError> {
        let core = Self::deferred(database_path.to_path_buf(), git);
        core.ensure_storage_ready()?;
        Ok(core)
    }

    pub fn deferred(database_path: PathBuf, git: GitProcess) -> Self {
        Self {
            project_inbox: Arc::new(ProjectInbox::deferred_with_git(database_path, git)),
        }
    }

    pub fn from_project_inbox(project_inbox: ProjectInbox) -> Self {
        Self {
            project_inbox: Arc::new(project_inbox),
        }
    }

    pub fn ensure_storage_ready(&self) -> Result<(), ApplicationError> {
        self.project_inbox
            .ensure_storage_ready()
            .map_err(ApplicationError::ProjectInbox)
    }

    pub fn project_inbox(&self) -> Arc<ProjectInbox> {
        Arc::clone(&self.project_inbox)
    }
}

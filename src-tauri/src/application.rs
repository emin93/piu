use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use thiserror::Error;

use crate::{
    chat_workspaces::{ChatWorkspaceError, ChatWorkspaces},
    git_process::GitProcess,
    project_inbox::{ProjectInbox, ProjectInboxError},
};

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    ProjectInbox(#[from] ProjectInboxError),
    #[error(transparent)]
    ChatWorkspaces(#[from] ChatWorkspaceError),
}

pub struct ApplicationCore {
    project_inbox: Arc<ProjectInbox>,
    chat_workspaces: Arc<ChatWorkspaces>,
}

impl ApplicationCore {
    pub fn open(database_path: &Path, git: GitProcess) -> Result<Self, ApplicationError> {
        let core = Self::deferred(database_path.to_path_buf(), git);
        core.ensure_storage_ready()?;
        Ok(core)
    }

    pub fn deferred(database_path: PathBuf, git: GitProcess) -> Self {
        let app_data = database_path
            .parent()
            .expect("application database always has an application data parent")
            .to_path_buf();
        let project_inbox = Arc::new(ProjectInbox::deferred_with_git(database_path, git.clone()));
        let chat_workspaces = Arc::new(ChatWorkspaces::new(
            Arc::clone(&project_inbox),
            git,
            app_data.join("worktrees"),
        ));
        Self {
            project_inbox,
            chat_workspaces,
        }
    }

    pub fn from_project_inbox(
        project_inbox: ProjectInbox,
        app_data: &Path,
        git: GitProcess,
    ) -> Self {
        let project_inbox = Arc::new(project_inbox);
        Self {
            chat_workspaces: Arc::new(ChatWorkspaces::new(
                Arc::clone(&project_inbox),
                git,
                app_data.join("worktrees"),
            )),
            project_inbox,
        }
    }

    pub fn ensure_storage_ready(&self) -> Result<(), ApplicationError> {
        self.project_inbox
            .ensure_storage_ready()
            .map_err(ApplicationError::ProjectInbox)?;
        self.chat_workspaces.reconcile_once()?;
        Ok(())
    }

    pub fn project_inbox(&self) -> Arc<ProjectInbox> {
        Arc::clone(&self.project_inbox)
    }

    pub fn chat_workspaces(&self) -> Arc<ChatWorkspaces> {
        Arc::clone(&self.chat_workspaces)
    }
}

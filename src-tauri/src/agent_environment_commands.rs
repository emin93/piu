use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::{
    agent_environment::{
        AgentEnvironment, AgentEnvironmentError, AgentEnvironmentSnapshot, AgentResourceId,
        AgentResourcePreferenceChange, AgentResourcePreferenceScope,
    },
    chat_runtime_host::{ModelControlsSnapshot, ModelRouteId, ReasoningEffort},
    project_inbox::ProjectInboxError,
};

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ProjectAgentEnvironmentRequest {
    #[ts(type = "number")]
    pub project_id: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct SelectProjectModelRouteRequest {
    #[ts(type = "number")]
    pub project_id: i64,
    pub route: ModelRouteId,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct SelectProjectReasoningEffortRequest {
    #[ts(type = "number")]
    pub project_id: i64,
    pub effort: ReasoningEffort,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct SetAgentResourceEnabledRequest {
    #[ts(type = "number")]
    pub project_id: i64,
    pub scope: AgentResourcePreferenceScope,
    pub resource: AgentResourceId,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum AgentEnvironmentCommandErrorCode {
    ProjectNotFound,
    ModelUnavailable,
    EffortUnavailable,
    ResourceUnavailable,
    LastModelRouteRequired,
    InspectionFailed,
    StorageUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AgentEnvironmentCommandError {
    pub code: AgentEnvironmentCommandErrorCode,
    pub message: String,
}

impl From<AgentEnvironmentError> for AgentEnvironmentCommandError {
    fn from(error: AgentEnvironmentError) -> Self {
        match error {
            AgentEnvironmentError::Project(ProjectInboxError::ProjectNotFound { .. }) => Self {
                code: AgentEnvironmentCommandErrorCode::ProjectNotFound,
                message: "That project is no longer in Più.".into(),
            },
            AgentEnvironmentError::ModelUnavailable { .. }
            | AgentEnvironmentError::NoAvailableModelRoutes => Self {
                code: AgentEnvironmentCommandErrorCode::ModelUnavailable,
                message: "That model is no longer available. Choose another model.".into(),
            },
            AgentEnvironmentError::EffortUnavailable { .. } => Self {
                code: AgentEnvironmentCommandErrorCode::EffortUnavailable,
                message: "That reasoning effort is unavailable for this model.".into(),
            },
            AgentEnvironmentError::ResourceUnavailable => Self {
                code: AgentEnvironmentCommandErrorCode::ResourceUnavailable,
                message: "That resource is no longer available in this project.".into(),
            },
            AgentEnvironmentError::CannotDisableLastModelRoute => Self {
                code: AgentEnvironmentCommandErrorCode::LastModelRouteRequired,
                message: "Keep at least one model enabled for new chats.".into(),
            },
            AgentEnvironmentError::NonAbsoluteProcessPath
            | AgentEnvironmentError::MissingHome
            | AgentEnvironmentError::InvalidPolicy
            | AgentEnvironmentError::Spawn
            | AgentEnvironmentError::TimedOut
            | AgentEnvironmentError::OutputLimitExceeded
            | AgentEnvironmentError::ChildFailed
            | AgentEnvironmentError::InvalidSnapshot => Self {
                code: AgentEnvironmentCommandErrorCode::InspectionFailed,
                message: "Più couldn’t inspect this project’s agent environment. Try again.".into(),
            },
            AgentEnvironmentError::RuntimeStorage
            | AgentEnvironmentError::Project(_)
            | AgentEnvironmentError::Preferences(_) => Self {
                code: AgentEnvironmentCommandErrorCode::StorageUnavailable,
                message: "Più couldn’t save the agent environment. Try again.".into(),
            },
        }
    }
}

#[tauri::command]
pub async fn get_project_agent_environment(
    environment: State<'_, AgentEnvironment>,
    request: ProjectAgentEnvironmentRequest,
) -> Result<AgentEnvironmentSnapshot, AgentEnvironmentCommandError> {
    environment
        .snapshot(request.project_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_project_model_controls(
    environment: State<'_, AgentEnvironment>,
    request: ProjectAgentEnvironmentRequest,
) -> Result<ModelControlsSnapshot, AgentEnvironmentCommandError> {
    environment
        .model_controls(request.project_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn select_project_model_route(
    environment: State<'_, AgentEnvironment>,
    request: SelectProjectModelRouteRequest,
) -> Result<ModelControlsSnapshot, AgentEnvironmentCommandError> {
    environment
        .select_model_route(request.project_id, request.route)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn select_project_reasoning_effort(
    environment: State<'_, AgentEnvironment>,
    request: SelectProjectReasoningEffortRequest,
) -> Result<ModelControlsSnapshot, AgentEnvironmentCommandError> {
    environment
        .select_reasoning_effort(request.project_id, request.effort)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_agent_resource_enabled(
    environment: State<'_, AgentEnvironment>,
    request: SetAgentResourceEnabledRequest,
) -> Result<AgentResourcePreferenceChange, AgentEnvironmentCommandError> {
    environment
        .set_resource_enabled(
            request.project_id,
            request.scope,
            request.resource,
            request.enabled,
        )
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_inspection_failures_cross_a_sanitized_typed_boundary() {
        for error in [
            AgentEnvironmentError::TimedOut,
            AgentEnvironmentError::OutputLimitExceeded,
            AgentEnvironmentError::InvalidSnapshot,
        ] {
            let boundary = AgentEnvironmentCommandError::from(error);
            assert_eq!(
                boundary.code,
                AgentEnvironmentCommandErrorCode::InspectionFailed
            );
            assert_eq!(
                boundary.message,
                "Più couldn’t inspect this project’s agent environment. Try again."
            );
        }
    }
}

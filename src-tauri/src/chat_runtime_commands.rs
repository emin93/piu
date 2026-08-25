use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, State};
use tokio::sync::broadcast::error::RecvError;
use ts_rs::TS;

use crate::{
    chat_runtime_host::{
        ChatRuntimeChangedEvent, ChatRuntimeHost, ChatRuntimeHostError, ConversationInputAnswer,
        ConversationSnapshot, ModelControlsSnapshot, ModelRouteId, ReasoningEffort,
    },
    chat_workspaces::ChatWorkspaceError,
    pi_rpc::PiRpcError,
    project_inbox::ProjectInboxError,
    prompt_attachments::{PromptAttachment, PromptAttachmentError},
};

pub const CHAT_RUNTIME_CHANGED_EVENT: &str = "chat-runtime://changed";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct OpenChatRuntimeRequest {
    pub chat_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct SelectModelRouteRequest {
    pub chat_id: String,
    pub route: ModelRouteId,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct SelectReasoningEffortRequest {
    pub chat_id: String,
    pub effort: ReasoningEffort,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ConversationStreamingBehavior {
    Steer,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ConversationPromptRequest {
    pub chat_id: String,
    pub streaming_behavior: ConversationStreamingBehavior,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatRuntimeMessageRequest {
    pub chat_id: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AnswerConversationInputRequest {
    pub chat_id: String,
    pub request_id: String,
    pub answer: ConversationInputAnswer,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum ChatRuntimeCommandErrorCode {
    EmptyMessage,
    ChatNotFound,
    SetupIncomplete,
    NotActive,
    RuntimeUnavailable,
    AuthenticationRequired,
    ConversationFailed,
    StorageUnavailable,
    InvalidAttachment,
    ModelMediaUnsupported,
    InputNotPending,
    InvalidInputAnswer,
    ModelUnavailable,
    EffortUnavailable,
    InferenceChangeRejected,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ChatRuntimeCommandError {
    pub code: ChatRuntimeCommandErrorCode,
    pub message: String,
}

impl From<ChatRuntimeHostError> for ChatRuntimeCommandError {
    fn from(error: ChatRuntimeHostError) -> Self {
        match error {
            ChatRuntimeHostError::EmptyMessage => Self {
                code: ChatRuntimeCommandErrorCode::EmptyMessage,
                message: "Write a message before sending it.".into(),
            },
            ChatRuntimeHostError::NotActive { .. } => Self {
                code: ChatRuntimeCommandErrorCode::NotActive,
                message: "Open the chat before sending that action.".into(),
            },
            ChatRuntimeHostError::InputNotPending { .. } => Self {
                code: ChatRuntimeCommandErrorCode::InputNotPending,
                message: "That question is no longer waiting for an answer.".into(),
            },
            ChatRuntimeHostError::InvalidInputAnswer => Self {
                code: ChatRuntimeCommandErrorCode::InvalidInputAnswer,
                message: "Choose one of the answers shown by Pi.".into(),
            },
            ChatRuntimeHostError::ModelUnavailable { .. } => Self {
                code: ChatRuntimeCommandErrorCode::ModelUnavailable,
                message: "That model is no longer available. Choose another model.".into(),
            },
            ChatRuntimeHostError::EffortUnavailable { .. } => Self {
                code: ChatRuntimeCommandErrorCode::EffortUnavailable,
                message: "That reasoning effort is unavailable for this model.".into(),
            },
            ChatRuntimeHostError::InferenceChangeRejected => Self {
                code: ChatRuntimeCommandErrorCode::InferenceChangeRejected,
                message: "Pi couldn’t switch models. The previous model is still selected.".into(),
            },
            ChatRuntimeHostError::SetupIncomplete { .. } => Self {
                code: ChatRuntimeCommandErrorCode::SetupIncomplete,
                message: "Finish the repository setup before starting the agent.".into(),
            },
            ChatRuntimeHostError::Attachment(PromptAttachmentError::ModelMediaUnsupported) => {
                Self {
                    code: ChatRuntimeCommandErrorCode::ModelMediaUnsupported,
                    message: "The selected model doesn’t accept image attachments.".into(),
                }
            }
            ChatRuntimeHostError::Attachment(_) => Self {
                code: ChatRuntimeCommandErrorCode::InvalidAttachment,
                message: "One of those attachments is no longer valid. Remove it and try again."
                    .into(),
            },
            ChatRuntimeHostError::Workspace(ChatWorkspaceError::Inbox(
                ProjectInboxError::ChatNotFound { .. },
            ))
            | ChatRuntimeHostError::Inbox(ProjectInboxError::ChatNotFound { .. }) => Self {
                code: ChatRuntimeCommandErrorCode::ChatNotFound,
                message: "That chat is no longer in Più.".into(),
            },
            ChatRuntimeHostError::Rpc(PiRpcError::Remote { command, message }) => {
                if command == "prompt" && is_codex_authentication_failure(&message) {
                    Self {
                        code: ChatRuntimeCommandErrorCode::AuthenticationRequired,
                        message: "Sign in to Codex to continue this conversation.".into(),
                    }
                } else {
                    Self {
                        code: ChatRuntimeCommandErrorCode::ConversationFailed,
                        message:
                            "Pi couldn’t accept that message. The conversation is still available."
                                .into(),
                    }
                }
            }
            ChatRuntimeHostError::Rpc(_)
            | ChatRuntimeHostError::Environment(_)
            | ChatRuntimeHostError::InvalidSessionState(_)
            | ChatRuntimeHostError::NonAbsolutePath
            | ChatRuntimeHostError::InvalidHome => Self {
                code: ChatRuntimeCommandErrorCode::RuntimeUnavailable,
                message:
                    "Più couldn’t start the bundled agent runtime. Try opening the chat again."
                        .into(),
            },
            ChatRuntimeHostError::RuntimeStorage(_)
            | ChatRuntimeHostError::Preferences(_)
            | ChatRuntimeHostError::Inbox(_)
            | ChatRuntimeHostError::Workspace(_)
            | ChatRuntimeHostError::Lock => Self {
                code: ChatRuntimeCommandErrorCode::StorageUnavailable,
                message: "Più couldn’t save this conversation. Try again.".into(),
            },
        }
    }
}

fn is_codex_authentication_failure(message: &str) -> bool {
    message.starts_with("Authentication failed for \"openai-codex\".")
        || message.starts_with("No API key found for openai-codex.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_codex_auth_failures_cross_the_typed_recovery_boundary() {
        for message in [
            "Authentication failed for \"openai-codex\". Credentials may have expired.",
            "No API key found for openai-codex.\n\nUse /login to log into a provider.",
        ] {
            let error =
                ChatRuntimeCommandError::from(ChatRuntimeHostError::Rpc(PiRpcError::Remote {
                    command: "prompt".into(),
                    message: message.into(),
                }));

            assert_eq!(
                error.code,
                ChatRuntimeCommandErrorCode::AuthenticationRequired
            );
            assert_eq!(
                error.message,
                "Sign in to Codex to continue this conversation."
            );
        }
    }

    #[test]
    fn unrelated_remote_failures_never_offer_authentication_as_recovery() {
        for (command, message) in [
            ("prompt", "fixture rejection"),
            (
                "get_state",
                "Authentication failed for \"openai-codex\". Credentials may have expired.",
            ),
        ] {
            let error =
                ChatRuntimeCommandError::from(ChatRuntimeHostError::Rpc(PiRpcError::Remote {
                    command: command.into(),
                    message: message.into(),
                }));

            assert_eq!(error.code, ChatRuntimeCommandErrorCode::ConversationFailed);
        }
    }
}

#[tauri::command]
pub async fn open_chat_runtime(
    host: State<'_, ChatRuntimeHost>,
    request: OpenChatRuntimeRequest,
) -> Result<ConversationSnapshot, ChatRuntimeCommandError> {
    host.open(&request.chat_id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn get_model_controls(
    host: State<'_, ChatRuntimeHost>,
    request: OpenChatRuntimeRequest,
) -> Result<ModelControlsSnapshot, ChatRuntimeCommandError> {
    host.model_controls(&request.chat_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn select_model_route(
    host: State<'_, ChatRuntimeHost>,
    request: SelectModelRouteRequest,
) -> Result<ModelControlsSnapshot, ChatRuntimeCommandError> {
    host.select_model_route(&request.chat_id, request.route)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn select_reasoning_effort(
    host: State<'_, ChatRuntimeHost>,
    request: SelectReasoningEffortRequest,
) -> Result<ModelControlsSnapshot, ChatRuntimeCommandError> {
    host.select_reasoning_effort(&request.chat_id, request.effort)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn send_chat_message(
    host: State<'_, ChatRuntimeHost>,
    request: ConversationPromptRequest,
) -> Result<(), ChatRuntimeCommandError> {
    host.send_with_attachments(&request.chat_id, &request.text, &request.attachments)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn steer_chat(
    host: State<'_, ChatRuntimeHost>,
    request: ChatRuntimeMessageRequest,
) -> Result<(), ChatRuntimeCommandError> {
    host.steer(&request.chat_id, &request.text)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn abort_chat_turn(
    host: State<'_, ChatRuntimeHost>,
    request: OpenChatRuntimeRequest,
) -> Result<(), ChatRuntimeCommandError> {
    host.abort(&request.chat_id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn answer_conversation_input(
    host: State<'_, ChatRuntimeHost>,
    request: AnswerConversationInputRequest,
) -> Result<(), ChatRuntimeCommandError> {
    host.answer_input(&request.chat_id, &request.request_id, request.answer)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn stop_chat_runtime(
    host: State<'_, ChatRuntimeHost>,
    request: OpenChatRuntimeRequest,
) -> Result<(), ChatRuntimeCommandError> {
    host.stop_runtime(&request.chat_id)
        .await
        .map_err(Into::into)
}

pub fn forward_chat_runtime_events<R: Runtime>(app: AppHandle<R>, host: &ChatRuntimeHost) {
    let mut events = host.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => emit_event(&app, event),
                Err(RecvError::Closed) => return,
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "chat runtime event forwarder fell behind");
                }
            }
        }
    });
}

fn emit_event<R: Runtime>(app: &AppHandle<R>, event: ChatRuntimeChangedEvent) {
    if let Err(error) = app.emit(CHAT_RUNTIME_CHANGED_EVENT, event) {
        tracing::warn!(%error, "could not emit chat runtime change");
    }
}

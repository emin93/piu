use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State, async_runtime};
use ts_rs::TS;

use crate::codex_auth::{CodexAuthError, CodexAuthManager, CodexAuthStatus};

pub const CODEX_AUTH_UPDATE_EVENT: &str = "codex-auth://update";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum CodexAuthErrorCode {
    AlreadyRunning,
    NotRunning,
    PromptNotPending,
    InvalidPromptResponse,
    SignInUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct CodexAuthCommandError {
    pub code: CodexAuthErrorCode,
    pub message: String,
}

impl From<CodexAuthError> for CodexAuthCommandError {
    fn from(error: CodexAuthError) -> Self {
        let code = match error {
            CodexAuthError::AlreadyRunning => CodexAuthErrorCode::AlreadyRunning,
            CodexAuthError::NotRunning => CodexAuthErrorCode::NotRunning,
            CodexAuthError::PromptNotPending => CodexAuthErrorCode::PromptNotPending,
            CodexAuthError::InvalidPromptResponse => CodexAuthErrorCode::InvalidPromptResponse,
            _ => CodexAuthErrorCode::SignInUnavailable,
        };
        let message = match code {
            CodexAuthErrorCode::AlreadyRunning => "Sign-in is already running.",
            CodexAuthErrorCode::NotRunning => "Sign-in is not running.",
            CodexAuthErrorCode::PromptNotPending => {
                "That sign-in prompt is no longer waiting for an answer."
            }
            CodexAuthErrorCode::InvalidPromptResponse => "Enter a valid response to continue.",
            CodexAuthErrorCode::SignInUnavailable => "Sign-in is unavailable. Try again.",
        }
        .to_owned();
        Self { code, message }
    }
}

#[tauri::command]
pub fn codex_auth_status(manager: State<'_, CodexAuthManager>) -> CodexAuthStatus {
    manager.status()
}

#[tauri::command]
pub async fn start_codex_sign_in(
    manager: State<'_, CodexAuthManager>,
) -> Result<CodexAuthStatus, CodexAuthCommandError> {
    manager.start().await.map_err(CodexAuthCommandError::from)?;
    Ok(manager.status())
}

#[tauri::command]
pub async fn answer_codex_auth_prompt(
    manager: State<'_, CodexAuthManager>,
    prompt_id: String,
    value: String,
) -> Result<(), CodexAuthCommandError> {
    manager
        .answer(&prompt_id, &value)
        .await
        .map_err(CodexAuthCommandError::from)
}

#[tauri::command]
pub async fn cancel_codex_sign_in(
    manager: State<'_, CodexAuthManager>,
) -> Result<(), CodexAuthCommandError> {
    manager.cancel().await.map_err(CodexAuthCommandError::from)
}

pub fn forward_updates<R: Runtime>(app: AppHandle<R>, manager: &CodexAuthManager) {
    let mut updates = manager.subscribe();
    let manager = manager.clone();
    async_runtime::spawn(async move {
        loop {
            match updates.recv().await {
                Ok(update) => {
                    if let Err(error) = app.emit(CODEX_AUTH_UPDATE_EVENT, update) {
                        tracing::warn!(%error, "could not emit Codex authentication update");
                    }
                }
                Err(CodexAuthError::UpdateBackpressure { .. }) => {
                    manager.fail_update_delivery();
                }
                Err(_) => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{CodexAuthCommandError, CodexAuthErrorCode};
    use crate::codex_auth::CodexAuthError;

    #[test]
    fn command_errors_keep_machine_readable_categories_and_safe_messages() {
        let prompt = CodexAuthCommandError::from(CodexAuthError::PromptNotPending);
        assert_eq!(prompt.code, CodexAuthErrorCode::PromptNotPending);
        assert_eq!(
            prompt.message,
            "That sign-in prompt is no longer waiting for an answer."
        );

        let unavailable = CodexAuthCommandError::from(CodexAuthError::Protocol);
        assert_eq!(unavailable.code, CodexAuthErrorCode::SignInUnavailable);
        assert_eq!(unavailable.message, "Sign-in is unavailable. Try again.");
    }
}

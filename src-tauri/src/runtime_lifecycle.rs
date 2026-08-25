use tauri::{AppHandle, Runtime, State};

use crate::{chat_runtime_host::ChatRuntimeHost, codex_auth::CodexAuthManager};

#[tauri::command]
pub fn has_active_agent_turn(chat_runtime: State<'_, ChatRuntimeHost>) -> Result<bool, String> {
    chat_runtime
        .has_active_turn()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn exit_application<R: Runtime>(app: AppHandle<R>) {
    app.exit(0);
}

#[tauri::command]
pub async fn shutdown_runtime_processes(
    chat_runtime: State<'_, ChatRuntimeHost>,
    codex_auth: State<'_, CodexAuthManager>,
) -> Result<(), String> {
    let ((), auth_result) = tokio::join!(chat_runtime.shutdown_all(), codex_auth.shutdown());
    if let Err(error) = auth_result {
        tracing::warn!(%error, "could not gracefully stop Codex sign-in");
    }
    Ok(())
}

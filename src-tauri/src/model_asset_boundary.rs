use serde::Serialize;
use tauri::{State, async_runtime};
use ts_rs::TS;

use crate::model_assets::{
    ModelAssetError, ModelAssetErrorCode, ModelAssetManager, ModelAssetStatus,
};

pub const MODEL_ASSET_STATUS_EVENT: &str = "model-assets://status";

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct ModelAssetCommandError {
    pub code: ModelAssetErrorCode,
    pub message: String,
}

impl From<ModelAssetError> for ModelAssetCommandError {
    fn from(error: ModelAssetError) -> Self {
        let message = error.to_string();
        let code = error.code();
        Self { code, message }
    }
}

#[tauri::command]
pub fn model_asset_status(manager: State<'_, ModelAssetManager>) -> ModelAssetStatus {
    manager.status()
}

#[tauri::command]
pub fn start_model_download(
    manager: State<'_, ModelAssetManager>,
) -> Result<u64, ModelAssetCommandError> {
    manager.start_download().map_err(Into::into)
}

#[tauri::command]
pub fn cancel_model_download(manager: State<'_, ModelAssetManager>) -> bool {
    manager.cancel_download()
}

#[tauri::command]
pub async fn authorize_hugging_face(
    manager: State<'_, ModelAssetManager>,
    token: String,
) -> Result<(), ModelAssetCommandError> {
    manager
        .authorize_hugging_face(token)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn remove_model_assets(
    manager: State<'_, ModelAssetManager>,
) -> Result<ModelAssetStatus, ModelAssetCommandError> {
    manager.remove_owned_assets().await.map_err(Into::into)
}

pub fn forward_status_events<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    manager: &ModelAssetManager,
) {
    use tauri::Emitter;

    let mut status = manager.subscribe();
    async_runtime::spawn(async move {
        while status.changed().await.is_ok() {
            if let Err(error) = app.emit(MODEL_ASSET_STATUS_EVENT, status.borrow().clone()) {
                tracing::warn!(%error, "could not emit model asset status");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::ModelAssetCommandError;
    use crate::model_assets::{ModelAssetError, ModelAssetErrorCode};

    #[test]
    fn command_errors_keep_machine_readable_categories() {
        let error = ModelAssetCommandError::from(ModelAssetError::InsufficientSpace {
            available: 4,
            required: 8,
        });

        assert!(matches!(error.code, ModelAssetErrorCode::InsufficientSpace));
        assert!(error.message.contains("4 bytes free"));
    }
}

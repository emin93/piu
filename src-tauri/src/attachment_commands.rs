use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::prompt_attachments::{PromptAttachment, PromptAttachmentError, prepare_files};

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct PreparePromptAttachmentsRequest {
    pub paths: Vec<String>,
    pub accepted: Vec<PromptAttachment>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum AttachmentCommandErrorCode {
    FolderNotSupported,
    Inaccessible,
    UnsupportedType,
    InvalidTextEncoding,
    Oversized,
    TooMany,
    TotalTooLarge,
    StorageUnavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AttachmentCommandError {
    pub code: AttachmentCommandErrorCode,
    pub message: String,
}

impl From<PromptAttachmentError> for AttachmentCommandError {
    fn from(error: PromptAttachmentError) -> Self {
        match error {
            PromptAttachmentError::FolderNotSupported => Self {
                code: AttachmentCommandErrorCode::FolderNotSupported,
                message: "Folders can’t be attached. Più can inspect the project directly.".into(),
            },
            PromptAttachmentError::Inaccessible => Self {
                code: AttachmentCommandErrorCode::Inaccessible,
                message:
                    "Più can’t access one of those files. Check its permissions and try again."
                        .into(),
            },
            PromptAttachmentError::UnsupportedType => Self {
                code: AttachmentCommandErrorCode::UnsupportedType,
                message: "Attach an image or an individual UTF-8 text file.".into(),
            },
            PromptAttachmentError::InvalidTextEncoding => Self {
                code: AttachmentCommandErrorCode::InvalidTextEncoding,
                message: "That text file isn’t UTF-8, so Più can’t attach it.".into(),
            },
            PromptAttachmentError::Oversized => Self {
                code: AttachmentCommandErrorCode::Oversized,
                message: "That file is too large to attach.".into(),
            },
            PromptAttachmentError::TooMany => Self {
                code: AttachmentCommandErrorCode::TooMany,
                message: "Remove an attachment before adding another.".into(),
            },
            PromptAttachmentError::TotalTooLarge => Self {
                code: AttachmentCommandErrorCode::TotalTooLarge,
                message: "Those files are too large to attach together.".into(),
            },
            PromptAttachmentError::ModelMediaUnsupported => Self {
                code: AttachmentCommandErrorCode::UnsupportedType,
                message: "The selected model doesn’t accept image attachments.".into(),
            },
        }
    }
}

#[tauri::command]
pub async fn prepare_prompt_attachments(
    request: PreparePromptAttachmentsRequest,
) -> Result<Vec<PromptAttachment>, AttachmentCommandError> {
    tauri::async_runtime::spawn_blocking(move || prepare_files(&request.paths, &request.accepted))
        .await
        .map_err(|_| AttachmentCommandError {
            code: AttachmentCommandErrorCode::StorageUnavailable,
            message: "Più couldn’t read those files. Try again.".into(),
        })?
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_failures_keep_a_typed_product_boundary() {
        let cases = [
            (
                PromptAttachmentError::FolderNotSupported,
                AttachmentCommandErrorCode::FolderNotSupported,
            ),
            (
                PromptAttachmentError::Inaccessible,
                AttachmentCommandErrorCode::Inaccessible,
            ),
            (
                PromptAttachmentError::InvalidTextEncoding,
                AttachmentCommandErrorCode::InvalidTextEncoding,
            ),
            (
                PromptAttachmentError::Oversized,
                AttachmentCommandErrorCode::Oversized,
            ),
            (
                PromptAttachmentError::TooMany,
                AttachmentCommandErrorCode::TooMany,
            ),
            (
                PromptAttachmentError::TotalTooLarge,
                AttachmentCommandErrorCode::TotalTooLarge,
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(AttachmentCommandError::from(source).code, expected);
        }
    }
}

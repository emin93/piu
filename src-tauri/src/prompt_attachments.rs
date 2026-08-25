use std::{collections::HashSet, fs, path::Path};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use ts_rs::TS;

const MAX_ATTACHMENT_COUNT: usize = 8;
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum PromptAttachmentKind {
    Image,
    Text,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct PromptAttachment {
    pub id: String,
    pub name: String,
    pub kind: PromptAttachmentKind,
    pub mime_type: String,
    /// Base64 for images and UTF-8 for text files.
    pub content: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum PromptAttachmentError {
    #[error("folders cannot be attached")]
    FolderNotSupported,
    #[error("the selected file cannot be accessed")]
    Inaccessible,
    #[error("the selected file is not an image or UTF-8 text file")]
    UnsupportedType,
    #[error("the selected text file is not valid UTF-8")]
    InvalidTextEncoding,
    #[error("the selected file is too large")]
    Oversized,
    #[error("too many files are attached")]
    TooMany,
    #[error("the attachments are too large together")]
    TotalTooLarge,
    #[error("the selected model does not accept image input")]
    ModelMediaUnsupported,
}

pub fn prepare_files(
    paths: &[String],
    accepted: &[PromptAttachment],
) -> Result<Vec<PromptAttachment>, PromptAttachmentError> {
    validate(accepted)?;
    let mut ids = accepted
        .iter()
        .map(|attachment| attachment.id.clone())
        .collect::<HashSet<_>>();
    let mut total_bytes = accepted.iter().map(|item| item.size_bytes).sum::<u64>();
    let mut prepared = Vec::new();

    for path in paths {
        let attachment = prepare_file(Path::new(path))?;
        if !ids.insert(attachment.id.clone()) {
            continue;
        }
        if accepted.len() + prepared.len() >= MAX_ATTACHMENT_COUNT {
            return Err(PromptAttachmentError::TooMany);
        }
        total_bytes = total_bytes
            .checked_add(attachment.size_bytes)
            .ok_or(PromptAttachmentError::TotalTooLarge)?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(PromptAttachmentError::TotalTooLarge);
        }
        prepared.push(attachment);
    }

    Ok(prepared)
}

pub fn validate(attachments: &[PromptAttachment]) -> Result<(), PromptAttachmentError> {
    if attachments.len() > MAX_ATTACHMENT_COUNT {
        return Err(PromptAttachmentError::TooMany);
    }
    let mut total_bytes = 0_u64;
    let mut ids = HashSet::new();
    for attachment in attachments {
        if attachment.id.is_empty() || attachment.name.is_empty() || !ids.insert(&attachment.id) {
            return Err(PromptAttachmentError::UnsupportedType);
        }
        let actual_size = match attachment.kind {
            PromptAttachmentKind::Image => {
                if !is_supported_image_mime(&attachment.mime_type) {
                    return Err(PromptAttachmentError::UnsupportedType);
                }
                let bytes = BASE64
                    .decode(&attachment.content)
                    .map_err(|_| PromptAttachmentError::UnsupportedType)?;
                if image_mime(&bytes) != Some(attachment.mime_type.as_str()) {
                    return Err(PromptAttachmentError::UnsupportedType);
                }
                u64::try_from(bytes.len()).map_err(|_| PromptAttachmentError::Oversized)?
            }
            PromptAttachmentKind::Text => {
                if attachment.mime_type != "text/plain" || attachment.content.contains('\0') {
                    return Err(PromptAttachmentError::UnsupportedType);
                }
                u64::try_from(attachment.content.len())
                    .map_err(|_| PromptAttachmentError::Oversized)?
            }
        };
        let limit = match attachment.kind {
            PromptAttachmentKind::Image => MAX_IMAGE_BYTES,
            PromptAttachmentKind::Text => MAX_TEXT_BYTES,
        };
        if actual_size != attachment.size_bytes || actual_size > limit {
            return Err(PromptAttachmentError::Oversized);
        }
        total_bytes = total_bytes
            .checked_add(actual_size)
            .ok_or(PromptAttachmentError::TotalTooLarge)?;
    }
    if total_bytes > MAX_TOTAL_BYTES {
        return Err(PromptAttachmentError::TotalTooLarge);
    }
    Ok(())
}

pub fn prompt_text(text: &str, attachments: &[PromptAttachment]) -> String {
    let mut result = text.trim().to_owned();
    for attachment in attachments
        .iter()
        .filter(|attachment| attachment.kind == PromptAttachmentKind::Text)
    {
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        result.push_str("--- BEGIN ATTACHED TEXT FILE ");
        result.push_str(&attachment.name.replace(['\r', '\n'], " "));
        result.push_str(" [");
        result.push_str(&attachment.id);
        result.push_str("] ---\n");
        result.push_str(&attachment.content);
        if !attachment.content.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("--- END ATTACHED TEXT FILE ");
        result.push_str(&attachment.name.replace(['\r', '\n'], " "));
        result.push_str(" [");
        result.push_str(&attachment.id);
        result.push_str("] ---");
    }
    if result.is_empty()
        && attachments
            .iter()
            .any(|attachment| attachment.kind == PromptAttachmentKind::Image)
    {
        result.push_str("Please inspect the attached image.");
    }
    result
}

pub fn image_payloads(attachments: &[PromptAttachment]) -> Vec<serde_json::Value> {
    attachments
        .iter()
        .filter(|attachment| attachment.kind == PromptAttachmentKind::Image)
        .map(|attachment| {
            serde_json::json!({
                "type": "image",
                "data": attachment.content,
                "mimeType": attachment.mime_type,
            })
        })
        .collect()
}

fn prepare_file(path: &Path) -> Result<PromptAttachment, PromptAttachmentError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| PromptAttachmentError::Inaccessible)?;
    if metadata.file_type().is_dir() {
        return Err(PromptAttachmentError::FolderNotSupported);
    }
    if !metadata.file_type().is_file() {
        return Err(PromptAttachmentError::UnsupportedType);
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(PromptAttachmentError::Oversized);
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or(PromptAttachmentError::UnsupportedType)?
        .to_owned();
    let bytes = fs::read(path).map_err(|_| PromptAttachmentError::Inaccessible)?;
    let size_bytes = u64::try_from(bytes.len()).map_err(|_| PromptAttachmentError::Oversized)?;
    let (kind, mime_type, content) = if let Some(mime_type) = image_mime(&bytes) {
        (
            PromptAttachmentKind::Image,
            mime_type.to_owned(),
            BASE64.encode(&bytes),
        )
    } else {
        if size_bytes > MAX_TEXT_BYTES {
            return Err(PromptAttachmentError::Oversized);
        }
        let content =
            String::from_utf8(bytes).map_err(|_| PromptAttachmentError::InvalidTextEncoding)?;
        if content.contains('\0') {
            return Err(PromptAttachmentError::UnsupportedType);
        }
        (PromptAttachmentKind::Text, "text/plain".into(), content)
    };
    let id = attachment_id(&name, kind, content.as_bytes());
    Ok(PromptAttachment {
        id,
        name,
        kind,
        mime_type,
        content,
        size_bytes,
    })
}

fn attachment_id(name: &str, kind: PromptAttachmentKind, content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(match kind {
        PromptAttachmentKind::Image => b"image".as_slice(),
        PromptAttachmentKind::Text => b"text".as_slice(),
    });
    hasher.update([0]);
    hasher.update(name.as_bytes());
    hasher.update([0]);
    hasher.update(content);
    let digest = hasher.finalize();
    let mut id = String::with_capacity("attachment-".len() + digest.len() * 2);
    id.push_str("attachment-");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut id, "{byte:02x}").expect("writing to a string cannot fail");
    }
    id
}

fn is_supported_image_mime(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn prepares_utf8_text_and_supported_images_as_immutable_values() {
        let root = tempdir().expect("tempdir");
        let text_path = root.path().join("notes.txt");
        let image_path = root.path().join("diagram.png");
        fs::write(&text_path, "hello π\n").expect("text fixture");
        fs::write(&image_path, b"\x89PNG\r\n\x1a\nfixture").expect("image fixture");

        let attachments = prepare_files(
            &[
                text_path.to_string_lossy().into_owned(),
                image_path.to_string_lossy().into_owned(),
            ],
            &[],
        )
        .expect("attachments");
        fs::write(&text_path, "changed").expect("mutate source");

        assert_eq!(attachments[0].kind, PromptAttachmentKind::Text);
        assert_eq!(attachments[0].content, "hello π\n");
        assert_eq!(attachments[1].kind, PromptAttachmentKind::Image);
        assert_eq!(
            BASE64.decode(&attachments[1].content).unwrap(),
            b"\x89PNG\r\n\x1a\nfixture"
        );
        validate(&attachments).expect("valid prepared values");
    }

    #[test]
    fn rejects_folders_inaccessible_files_non_utf8_files_and_oversized_text() {
        let root = tempdir().expect("tempdir");
        let missing_path = root.path().join("missing.txt");
        let binary_path = root.path().join("binary.dat");
        fs::write(&binary_path, [0xff, 0xfe]).expect("binary fixture");
        assert_eq!(
            prepare_files(&[root.path().to_string_lossy().into_owned()], &[]),
            Err(PromptAttachmentError::FolderNotSupported)
        );
        assert_eq!(
            prepare_files(&[missing_path.to_string_lossy().into_owned()], &[]),
            Err(PromptAttachmentError::Inaccessible)
        );
        assert_eq!(
            prepare_files(&[binary_path.to_string_lossy().into_owned()], &[]),
            Err(PromptAttachmentError::InvalidTextEncoding)
        );

        let large_path = root.path().join("large.txt");
        fs::write(
            &large_path,
            vec![b'a'; usize::try_from(MAX_TEXT_BYTES + 1).unwrap()],
        )
        .expect("large fixture");
        assert_eq!(
            prepare_files(&[large_path.to_string_lossy().into_owned()], &[]),
            Err(PromptAttachmentError::Oversized)
        );
    }

    #[test]
    fn text_content_is_unambiguously_delimited_and_images_use_pi_native_payloads() {
        let attachments = vec![
            PromptAttachment {
                id: "text-1".into(),
                name: "notes.txt".into(),
                kind: PromptAttachmentKind::Text,
                mime_type: "text/plain".into(),
                content: "first line".into(),
                size_bytes: 10,
            },
            PromptAttachment {
                id: "image-1".into(),
                name: "view.png".into(),
                kind: PromptAttachmentKind::Image,
                mime_type: "image/png".into(),
                content: BASE64.encode(b"\x89PNG\r\n\x1a\nfixture"),
                size_bytes: 15,
            },
        ];

        assert_eq!(
            prompt_text("Inspect these", &attachments),
            "Inspect these\n\n--- BEGIN ATTACHED TEXT FILE notes.txt [text-1] ---\nfirst line\n--- END ATTACHED TEXT FILE notes.txt [text-1] ---"
        );
        assert_eq!(
            image_payloads(&attachments),
            vec![serde_json::json!({
                "type": "image",
                "data": attachments[1].content,
                "mimeType": "image/png",
            })]
        );
        assert_eq!(
            prompt_text("", &attachments[1..]),
            "Please inspect the attached image."
        );
    }
}

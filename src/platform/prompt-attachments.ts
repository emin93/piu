import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import type { AttachmentCommandError } from "../generated/AttachmentCommandError";
import type { AttachmentCommandErrorCode } from "../generated/AttachmentCommandErrorCode";
import type { PromptAttachment } from "../generated/PromptAttachment";

export type { PromptAttachment } from "../generated/PromptAttachment";

const ATTACHMENT_ERROR_CODES = new Set<AttachmentCommandErrorCode>([
  "folderNotSupported",
  "inaccessible",
  "unsupportedType",
  "invalidTextEncoding",
  "oversized",
  "tooMany",
  "totalTooLarge",
  "storageUnavailable",
]);

export type PromptAttachmentSelection =
  { outcome: "cancelled" } | { attachments: PromptAttachment[]; outcome: "selected" };

export async function selectPromptAttachments(
  accepted: readonly PromptAttachment[],
): Promise<PromptAttachmentSelection> {
  const selection = await open({
    directory: false,
    multiple: true,
    title: "Attach Files",
  });
  if (selection === null) return { outcome: "cancelled" };
  const paths = typeof selection === "string" ? [selection] : selection;
  if (paths.length === 0) return { outcome: "cancelled" };
  const attachments = await invoke<PromptAttachment[]>("prepare_prompt_attachments", {
    request: { accepted, paths },
  });
  return { attachments, outcome: "selected" };
}

export function attachmentErrorMessage(error: unknown) {
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    ATTACHMENT_ERROR_CODES.has((error as AttachmentCommandError).code) &&
    "message" in error &&
    typeof (error as AttachmentCommandError).message === "string"
  ) {
    return (error as AttachmentCommandError).message;
  }
  return "Più couldn’t attach those files. Try again.";
}

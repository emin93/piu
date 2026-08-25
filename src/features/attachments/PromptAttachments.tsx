import { FileTextIcon, LoaderCircleIcon, PaperclipIcon, XIcon } from "lucide-react";
import { memo, useCallback, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  attachmentErrorMessage,
  selectPromptAttachments,
  type PromptAttachment,
} from "@/platform/prompt-attachments";

export const PromptAttachmentTray = memo(function PromptAttachmentTray({
  attachments,
  disabled = false,
  onRemove,
}: {
  attachments: readonly PromptAttachment[];
  disabled?: boolean;
  onRemove?: (attachmentId: string) => void;
}) {
  if (attachments.length === 0) return null;
  return (
    <ul aria-label="Attached files" className="prompt-attachment-tray">
      {attachments.map((attachment) => (
        <li className="prompt-attachment" key={attachment.id}>
          <span className="prompt-attachment-preview" data-kind={attachment.kind}>
            {attachment.kind === "image" ? (
              <img alt="" src={`data:${attachment.mimeType};base64,${attachment.content}`} />
            ) : (
              <FileTextIcon aria-hidden="true" />
            )}
          </span>
          <span className="prompt-attachment-name" title={attachment.name}>
            {attachment.name}
          </span>
          {onRemove ? (
            <Button
              aria-label={`Remove ${attachment.name}`}
              className="prompt-attachment-remove"
              disabled={disabled}
              onClick={() => onRemove(attachment.id)}
              size="icon-xs"
              type="button"
              variant="ghost"
            >
              <XIcon aria-hidden="true" />
            </Button>
          ) : null}
        </li>
      ))}
    </ul>
  );
});

export function PromptAttachmentButton({
  attachments,
  disabled = false,
  onChange,
  onError,
}: {
  attachments: readonly PromptAttachment[];
  disabled?: boolean;
  onChange: (attachments: PromptAttachment[]) => void;
  onError: (message: string | undefined) => void;
}) {
  const [selecting, setSelecting] = useState(false);
  const attach = useCallback(async () => {
    if (selecting) return;
    setSelecting(true);
    onError(undefined);
    try {
      const selection = await selectPromptAttachments(attachments);
      if (selection.outcome === "selected") {
        onChange([...attachments, ...selection.attachments]);
      }
    } catch (error: unknown) {
      onError(attachmentErrorMessage(error));
    } finally {
      setSelecting(false);
    }
  }, [attachments, onChange, onError, selecting]);

  return (
    <Button
      aria-label="Attach files"
      disabled={disabled || selecting}
      onClick={() => void attach()}
      size="icon-sm"
      title="Attach files"
      type="button"
      variant="ghost"
    >
      {selecting ? (
        <LoaderCircleIcon aria-hidden="true" className="attachment-spin" />
      ) : (
        <PaperclipIcon aria-hidden="true" />
      )}
    </Button>
  );
}

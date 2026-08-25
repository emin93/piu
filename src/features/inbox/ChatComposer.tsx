import { ArrowUpIcon, TriangleAlertIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

import { ProductComposer } from "@/components/ProductComposer";
import { Button } from "@/components/ui/button";
import {
  PromptAttachmentButton,
  PromptAttachmentTray,
} from "@/features/attachments/PromptAttachments";
import type { PromptAttachment } from "@/platform/prompt-attachments";
import type { ProjectSummary } from "@/platform/project-inbox";

import { ProjectDraftController, type DraftPersistenceStatus } from "./draft-controller";

interface ChatComposerProps {
  drafts: ProjectDraftController;
  layout?: "centered" | "docked";
  onSubmit?: (
    projectId: number,
    prompt: string,
    attachments: readonly PromptAttachment[],
  ) => Promise<string | undefined>;
  project: ProjectSummary;
}

function draftStatusCopy(status: DraftPersistenceStatus) {
  if (status.state === "saving") return "Saving";
  if (status.state === "saved") return "Saved locally";
  if (status.state === "failed") return "Not saved";
  return "Saved as you type";
}

export function ChatComposer({
  drafts,
  layout = "centered",
  onSubmit,
  project,
}: ChatComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const available = project.availability === "available";
  const subscribe = useCallback(
    (listener: () => void) => drafts.subscribe(project.id, listener),
    [drafts, project.id],
  );
  const getSnapshot = useCallback(() => drafts.get(project.id), [drafts, project.id]);
  const draft = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const hasRetainedDraft =
    !available && (Boolean(draft.prompt.trim()) || draft.attachments.length > 0);
  const [submitting, setSubmitting] = useState(false);
  const [submissionError, setSubmissionError] = useState<string>();
  const [attachmentError, setAttachmentError] = useState<string>();

  const submit = useCallback(async () => {
    const prompt = draft.prompt.trim();
    if (!onSubmit || (!prompt && draft.attachments.length === 0) || submitting) return;
    setSubmitting(true);
    setSubmissionError(undefined);
    const error = await onSubmit(project.id, prompt, draft.attachments);
    setSubmitting(false);
    if (error) setSubmissionError(error);
  }, [draft.attachments, draft.prompt, onSubmit, project.id, submitting]);

  const removeAttachment = useCallback(
    (attachmentId: string) => {
      drafts.changeAttachments(
        project.id,
        draft.attachments.filter((attachment) => attachment.id !== attachmentId),
      );
    },
    [draft.attachments, drafts, project.id],
  );

  useEffect(() => {
    if (!available) return;
    textareaRef.current?.focus({ preventScroll: true });
  }, [available, project.id]);

  return (
    <section
      aria-labelledby="new-chat-heading"
      className="composer-stage"
      data-composer-layout={layout}
      data-repository-available={available || undefined}
    >
      <div className="composer-heading">
        <h2 id="new-chat-heading">
          {available ? "Start a chat" : `${project.name} is unavailable`}
        </h2>
        {available ? (
          <p className="composer-context">
            New chat in <span title={project.name}>{project.name}</span>
          </p>
        ) : (
          <p role="alert">
            <TriangleAlertIcon aria-hidden="true" />
            Move the repository back or restore access before starting a chat.
          </p>
        )}
      </div>

      {available ? (
        <ProductComposer
          attachments={
            <PromptAttachmentTray
              attachments={draft.attachments}
              disabled={submitting}
              onRemove={removeAttachment}
            />
          }
          actions={
            <Button
              aria-label="Send message"
              disabled={
                !onSubmit || (!draft.prompt.trim() && draft.attachments.length === 0) || submitting
              }
              size="icon"
              type="submit"
            >
              <ArrowUpIcon aria-hidden="true" />
            </Button>
          }
          ariaDescribedBy="draft-persistence-status"
          ariaLabel={`Draft for ${project.name}`}
          error={
            draft.status.state === "failed" || submissionError || attachmentError
              ? {
                  action:
                    draft.status.state === "failed" ? (
                      <Button
                        onClick={() => void drafts.retry(project.id)}
                        size="sm"
                        type="button"
                        variant="outline"
                      >
                        Retry save
                      </Button>
                    ) : undefined,
                  message: (
                    <>
                      {draft.status.state === "failed" ? <span>{draft.status.message}</span> : null}
                      {submissionError ? <span>{submissionError}</span> : null}
                      {attachmentError ? <span>{attachmentError}</span> : null}
                    </>
                  ),
                }
              : undefined
          }
          inputRef={textareaRef}
          inputReadOnly={submitting}
          layout={layout}
          leadingActions={
            <PromptAttachmentButton
              attachments={draft.attachments}
              disabled={submitting}
              onChange={(attachments) => drafts.changeAttachments(project.id, attachments)}
              onError={setAttachmentError}
            />
          }
          onSubmit={() => void submit()}
          onValueChange={(value) => drafts.change(project.id, value)}
          placeholder="Describe what you want to change"
          status={
            <span
              aria-live="polite"
              className={draft.status.state === "failed" ? "text-destructive" : undefined}
              id="draft-persistence-status"
            >
              {draftStatusCopy(draft.status)}
            </span>
          }
          value={draft.prompt}
        />
      ) : hasRetainedDraft ? (
        <ProductComposer
          attachments={<PromptAttachmentTray attachments={draft.attachments} />}
          ariaLabel={`Retained draft for ${project.name}`}
          layout={layout}
          readOnly
          status={<span>Draft retained locally · read-only</span>}
          value={draft.prompt}
        />
      ) : null}
    </section>
  );
}

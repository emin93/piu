import { ArrowUpIcon, TriangleAlertIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import type { ProjectSummary } from "@/platform/project-inbox";

import { ProjectDraftController, type DraftPersistenceStatus } from "./draft-controller";

interface ChatComposerProps {
  drafts: ProjectDraftController;
  layout?: "centered" | "docked";
  onSubmit?: (projectId: number, prompt: string) => Promise<string | undefined>;
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
  const hasRetainedDraft = !available && Boolean(draft.prompt.trim());
  const [submitting, setSubmitting] = useState(false);
  const [submissionError, setSubmissionError] = useState<string>();

  const submit = useCallback(async () => {
    const prompt = draft.prompt.trim();
    if (!onSubmit || !prompt || submitting) return;
    setSubmitting(true);
    setSubmissionError(undefined);
    const error = await onSubmit(project.id, prompt);
    setSubmitting(false);
    if (error) setSubmissionError(error);
  }, [draft.prompt, onSubmit, project.id, submitting]);

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
        <form
          className="composer-shell"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <Textarea
            aria-describedby="draft-persistence-status"
            aria-label={`Draft for ${project.name}`}
            className="composer-input"
            onChange={(event) => drafts.change(project.id, event.target.value)}
            placeholder="Describe what you want to change"
            ref={textareaRef}
            rows={4}
            value={draft.prompt}
          />
          <div className="composer-footer">
            <span
              aria-live="polite"
              className={draft.status.state === "failed" ? "text-destructive" : undefined}
              id="draft-persistence-status"
            >
              {draftStatusCopy(draft.status)}
            </span>
            <Button
              aria-label="Send message"
              disabled={!onSubmit || !draft.prompt.trim() || submitting}
              size="icon"
              type="submit"
            >
              <ArrowUpIcon aria-hidden="true" />
            </Button>
          </div>
          {draft.status.state === "failed" ? (
            <div className="composer-error" role="alert">
              <span>{draft.status.message}</span>
              <Button onClick={() => void drafts.retry(project.id)} size="sm" variant="outline">
                Retry save
              </Button>
            </div>
          ) : null}
          {submissionError ? (
            <div className="composer-error" role="alert">
              <span>{submissionError}</span>
            </div>
          ) : null}
        </form>
      ) : hasRetainedDraft ? (
        <div className="composer-shell composer-shell-readonly">
          <Textarea
            aria-label={`Retained draft for ${project.name}`}
            className="composer-input"
            readOnly
            rows={4}
            value={draft.prompt}
          />
          <div className="composer-footer">
            <span>Draft retained locally · read-only</span>
          </div>
        </div>
      ) : null}
    </section>
  );
}

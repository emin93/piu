import { useCallback, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  conversationErrorMessage,
  type ConversationInputAnswer,
  type ConversationInputRequest,
} from "@/platform/conversations";

interface ConversationInputDialogProps {
  onAnswer: (answer: ConversationInputAnswer) => Promise<void>;
  request: ConversationInputRequest;
}

function ConversationInputDialogState({ onAnswer, request }: ConversationInputDialogProps) {
  const [value, setValue] = useState(request.prefill ?? "");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const inputRef = useRef<HTMLInputElement>(null);
  const editorRef = useRef<HTMLTextAreaElement>(null);
  const optionRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const initialFocus =
    request.kind === "input"
      ? inputRef
      : request.kind === "editor"
        ? editorRef
        : request.kind === "confirm"
          ? confirmRef
          : optionRef;

  const answer = useCallback(
    async (next: ConversationInputAnswer) => {
      if (pending) return;
      setPending(true);
      setError(undefined);
      try {
        await onAnswer(next);
      } catch (answerError: unknown) {
        setError(
          conversationErrorMessage(
            answerError,
            "Più couldn’t send that answer. The question is still available.",
          ),
        );
        setPending(false);
      }
    },
    [onAnswer, pending],
  );

  const cancel = useCallback(() => void answer({ kind: "cancelled" }), [answer]);

  return (
    <Dialog onOpenChange={(open) => (!open && !pending ? cancel() : undefined)} open>
      <DialogContent initialFocus={initialFocus} showCloseButton={!pending}>
        <DialogHeader>
          <DialogTitle>{request.title || "Pi needs input"}</DialogTitle>
          <DialogDescription>
            {request.message || "Answer this question to let the current turn continue."}
          </DialogDescription>
        </DialogHeader>

        {request.kind === "select" ? (
          <div className="conversation-input-options">
            {request.options.map((option, index) => (
              <Button
                disabled={pending}
                key={option}
                onClick={() => void answer({ kind: "value", value: option })}
                ref={index === 0 ? optionRef : undefined}
                type="button"
                variant="outline"
              >
                {option}
              </Button>
            ))}
          </div>
        ) : request.kind === "input" ? (
          <Input
            aria-label={request.title || "Answer"}
            disabled={pending}
            onChange={(event) => setValue(event.target.value)}
            placeholder={request.placeholder ?? undefined}
            ref={inputRef}
            value={value}
          />
        ) : request.kind === "editor" ? (
          <Textarea
            aria-label={request.title || "Answer"}
            disabled={pending}
            onChange={(event) => setValue(event.target.value)}
            placeholder={request.placeholder ?? undefined}
            ref={editorRef}
            rows={6}
            value={value}
          />
        ) : null}

        {error ? <p role="alert">{error}</p> : null}

        {request.kind === "confirm" ? (
          <DialogFooter>
            <Button disabled={pending} onClick={cancel} type="button" variant="ghost">
              Cancel
            </Button>
            <Button
              disabled={pending}
              onClick={() => void answer({ confirmed: false, kind: "confirmed" })}
              type="button"
              variant="outline"
            >
              No
            </Button>
            <Button
              disabled={pending}
              onClick={() => void answer({ confirmed: true, kind: "confirmed" })}
              ref={confirmRef}
              type="button"
            >
              Yes
            </Button>
          </DialogFooter>
        ) : request.kind === "input" || request.kind === "editor" ? (
          <DialogFooter>
            <Button disabled={pending} onClick={cancel} type="button" variant="ghost">
              Cancel
            </Button>
            <Button
              disabled={pending}
              onClick={() => void answer({ kind: "value", value })}
              type="button"
            >
              {pending ? "Submitting…" : "Submit"}
            </Button>
          </DialogFooter>
        ) : (
          <DialogFooter>
            <Button disabled={pending} onClick={cancel} type="button" variant="ghost">
              Cancel
            </Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}

export function ConversationInputDialog(props: ConversationInputDialogProps) {
  return <ConversationInputDialogState key={props.request.id} {...props} />;
}

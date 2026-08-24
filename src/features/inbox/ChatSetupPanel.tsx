import {
  CircleCheckIcon,
  LoaderCircleIcon,
  RotateCcwIcon,
  SquareTerminalIcon,
  XIcon,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { ChatSummary } from "@/platform/project-inbox";

import { ChatSetupController } from "./setup-controller";

interface ChatSetupPanelProps {
  chat: ChatSummary;
  onCancel: (chatId: string) => Promise<string | undefined>;
  onOpenTerminal: (chatId: string) => Promise<string | undefined>;
  onRetry: (chatId: string) => Promise<string | undefined>;
  setups: ChatSetupController;
}

type SetupAction = "cancel" | "retry" | "terminal";

function failureCopy(chat: ChatSummary, setup: ChatSummary["setup"]) {
  if (setup.phase === "cancelled") return "Setup was cancelled. The worktree is still available.";
  if (setup.failure === "notExecutable") {
    return "Make .piu/setup.sh executable, then retry.";
  }
  if (setup.failure === "exit" && setup.exitCode !== null) {
    return `.piu/setup.sh exited with code ${setup.exitCode}.`;
  }
  if (setup.failure === "signal" && setup.signal !== null) {
    return `.piu/setup.sh stopped after signal ${setup.signal}.`;
  }
  if (setup.failure === "interrupted") {
    return "Più closed before setup finished. Retry from the preserved worktree.";
  }
  return `Più couldn’t finish setup for ${chat.projectName}.`;
}

export function ChatSetupPanel({
  chat,
  onCancel,
  onOpenTerminal,
  onRetry,
  setups,
}: ChatSetupPanelProps) {
  const subscribe = useCallback(
    (listener: () => void) => setups.subscribe(chat.id, listener),
    [chat.id, setups],
  );
  const getSnapshot = useCallback(() => setups.get(chat.id), [chat.id, setups]);
  const setup = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const [pendingAction, setPendingAction] = useState<SetupAction>();
  const [actionError, setActionError] = useState<string>();
  const statusHeadingRef = useRef<HTMLHeadingElement>(null);
  const previousSetupRef = useRef({ chatId: chat.id, phase: setup.phase });

  const runAction = useCallback(
    async (action: SetupAction) => {
      setPendingAction(action);
      setActionError(undefined);
      const error = await (action === "retry"
        ? onRetry(chat.id)
        : action === "cancel"
          ? onCancel(chat.id)
          : onOpenTerminal(chat.id));
      setPendingAction(undefined);
      if (error) setActionError(error);
    },
    [chat.id, onCancel, onOpenTerminal, onRetry],
  );

  const running = setup.phase === "pending" || setup.phase === "running";
  const failed = setup.phase === "failed" || setup.phase === "cancelled";
  const ready = setup.phase === "notRequired" || setup.phase === "succeeded";

  useEffect(() => {
    const previousSetup = previousSetupRef.current;
    previousSetupRef.current = { chatId: chat.id, phase: setup.phase };
    const wasRunning = previousSetup.phase === "pending" || previousSetup.phase === "running";
    if (previousSetup.chatId === chat.id && wasRunning && !running) {
      statusHeadingRef.current?.focus({ preventScroll: true });
    }
  }, [chat.id, running, setup.phase]);

  return (
    <section aria-busy={running} aria-labelledby="setup-stage-title" className="setup-stage">
      <header aria-atomic="true" aria-live="polite" className="setup-stage-header" role="status">
        <div className="setup-stage-status" data-phase={setup.phase}>
          {running ? (
            <LoaderCircleIcon aria-hidden="true" className="setup-spinner" />
          ) : ready ? (
            <CircleCheckIcon aria-hidden="true" />
          ) : (
            <XIcon aria-hidden="true" />
          )}
        </div>
        <div>
          <p className="setup-stage-project">{chat.projectName}</p>
          <h2 id="setup-stage-title" ref={statusHeadingRef} tabIndex={-1}>
            {running ? "Setting up worktree" : ready ? "Worktree ready" : "Setup failed"}
          </h2>
          <p className="setup-stage-copy">
            {running
              ? "Running the repository setup before the agent starts."
              : ready
                ? "The isolated chat worktree is ready."
                : failureCopy(chat, setup)}
          </p>
        </div>
      </header>

      {setup.log ? (
        <ScrollArea className="setup-log-scroll">
          <pre aria-label="Setup output" className="setup-log">
            {setup.log}
          </pre>
        </ScrollArea>
      ) : running ? (
        <div className="setup-log-empty">Waiting for setup output</div>
      ) : null}

      {actionError ? (
        <p className="setup-action-error" role="alert">
          {actionError}
        </p>
      ) : null}

      <footer className="setup-stage-actions">
        {running ? (
          <Button
            disabled={Boolean(pendingAction)}
            onClick={() => void runAction("cancel")}
            type="button"
            variant="outline"
          >
            {pendingAction === "cancel" ? "Cancelling" : "Cancel setup"}
          </Button>
        ) : null}
        {failed ? (
          <>
            <Button
              disabled={Boolean(pendingAction)}
              onClick={() => void runAction("retry")}
              type="button"
            >
              <RotateCcwIcon aria-hidden="true" />
              {pendingAction === "retry" ? "Retrying" : "Retry setup"}
            </Button>
            <Button
              disabled={Boolean(pendingAction)}
              onClick={() => void runAction("terminal")}
              type="button"
              variant="outline"
            >
              <SquareTerminalIcon aria-hidden="true" />
              Open Terminal
            </Button>
          </>
        ) : null}
      </footer>
    </section>
  );
}

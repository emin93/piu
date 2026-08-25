import {
  CheckCircle2Icon,
  CheckIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  CopyIcon,
  ExternalLinkIcon,
  KeyRoundIcon,
  LoaderCircleIcon,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useEffectEvent,
  useId,
  useRef,
  useState,
  type FormEvent,
} from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type {
  CodexAuthAdapter,
  CodexAuthNotification,
  CodexAuthPrompt,
  CodexAuthRecord,
  CodexAuthSession,
} from "@/platform/codex-auth";

import "./codex-sign-in.css";

interface CodexSignInProps {
  adapter: CodexAuthAdapter;
  onComplete?: () => void;
}

interface ActivePrompt {
  id: string;
  prompt: CodexAuthPrompt;
}

interface AuthNotificationItem {
  id: number;
  notification: CodexAuthNotification;
}

type AuthPhase = "connecting" | "active" | "cancelling" | "complete" | "cancelled" | "failed";
type RunSessionAction = (action: (session: CodexAuthSession) => Promise<void>) => Promise<boolean>;

function AuthSpinner() {
  return (
    <span aria-hidden="true" className="codex-auth__spinner">
      <LoaderCircleIcon />
    </span>
  );
}

function NotificationView({
  copiedCode,
  item,
  onCodeCopied,
  runSessionAction,
}: {
  copiedCode?: string;
  item: AuthNotificationItem;
  onCodeCopied: (code: string) => void;
  runSessionAction: RunSessionAction;
}) {
  const notification = item.notification;
  if (notification.type === "info") {
    return (
      <section className="codex-auth__notice" data-kind="info">
        <p>{notification.message}</p>
        {notification.links ? (
          <div className="codex-auth__inline-actions">
            {notification.links.map((link) => (
              <Button
                key={`${link.url}-${link.label ?? "link"}`}
                onClick={() => void runSessionAction((session) => session.openExternal(link.url))}
                type="button"
                variant="ghost"
              >
                <ExternalLinkIcon aria-hidden="true" />
                {link.label ?? "Open link"}
              </Button>
            ))}
          </div>
        ) : null}
      </section>
    );
  }

  if (notification.type === "auth_url") {
    return (
      <section className="codex-auth__notice" data-kind="browser">
        <p>{notification.instructions ?? "Continue sign-in in your browser."}</p>
        <Button
          onClick={() => void runSessionAction((session) => session.openExternal(notification.url))}
          type="button"
          variant="outline"
        >
          <ExternalLinkIcon aria-hidden="true" />
          Open sign-in page
        </Button>
      </section>
    );
  }

  if (notification.type === "device_code") {
    const expiryMinutes = notification.expiresInSeconds
      ? Math.ceil(notification.expiresInSeconds / 60)
      : undefined;
    const expiry = expiryMinutes
      ? `Expires in ${expiryMinutes} ${expiryMinutes === 1 ? "minute" : "minutes"}`
      : undefined;
    const copied = copiedCode === notification.userCode;
    return (
      <section className="codex-auth__device">
        <div className="codex-auth__device-code">
          <code>{notification.userCode}</code>
          {expiry ? <p>{expiry}</p> : null}
        </div>
        <div className="codex-auth__device-actions">
          <Button
            onClick={() =>
              void runSessionAction((session) => session.copyText(notification.userCode)).then(
                (didCopy) => {
                  if (didCopy) onCodeCopied(notification.userCode);
                },
              )
            }
            type="button"
            variant="outline"
          >
            {copied ? <CheckIcon aria-hidden="true" /> : <CopyIcon aria-hidden="true" />}
            {copied ? "Copied" : "Copy code"}
          </Button>
          <Button
            onClick={() =>
              void runSessionAction((session) => session.openExternal(notification.verificationUri))
            }
            type="button"
            variant="outline"
          >
            <ExternalLinkIcon aria-hidden="true" />
            Open verification page
          </Button>
        </div>
      </section>
    );
  }

  return (
    <div className="codex-auth__progress" role="status">
      <AuthSpinner />
      <p>{notification.message}</p>
    </div>
  );
}

export function CodexSignIn({ adapter, onComplete }: CodexSignInProps) {
  const fieldId = useId();
  const sessionRef = useRef<CodexAuthSession | undefined>(undefined);
  const terminalRef = useRef(false);
  const notificationSequence = useRef(0);
  const [prompt, setPrompt] = useState<ActivePrompt>();
  const promptRef = useRef<ActivePrompt | undefined>(undefined);
  const [notifications, setNotifications] = useState<readonly AuthNotificationItem[]>([]);
  const [actionError, setActionError] = useState<string>();
  const [pendingPromptId, setPendingPromptId] = useState<string>();
  const [copiedCode, setCopiedCode] = useState<string>();
  const [phase, setPhase] = useState<AuthPhase>("connecting");
  const [terminalMessage, setTerminalMessage] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  const notifyComplete = useEffectEvent(() => onComplete?.());

  useEffect(() => {
    let active = true;
    let session: CodexAuthSession | undefined;
    terminalRef.current = false;
    sessionRef.current = undefined;
    promptRef.current = undefined;
    const receive = (record: CodexAuthRecord) => {
      if (!active) return;
      if (record.type === "auth_prompt") {
        const nextPrompt = { id: record.id, prompt: record.prompt };
        promptRef.current = nextPrompt;
        setPrompt(nextPrompt);
        setPendingPromptId(undefined);
        setActionError(undefined);
      } else if (record.type === "auth_event") {
        const item = { id: ++notificationSequence.current, notification: record.event };
        setNotifications((current) =>
          record.event.type === "progress"
            ? [...current.filter(({ notification }) => notification.type !== "progress"), item]
            : [...current, item],
        );
        if (record.event.type === "device_code") setCopiedCode(undefined);
      } else if (record.type === "auth_prompt_cancelled" && promptRef.current?.id === record.id) {
        promptRef.current = undefined;
        setPrompt(undefined);
        setPendingPromptId(undefined);
      } else if (record.type === "auth_complete") {
        terminalRef.current = true;
        promptRef.current = undefined;
        setPrompt(undefined);
        setActionError(undefined);
        setPendingPromptId(undefined);
        setPhase("complete");
        notifyComplete();
      } else if (record.type === "auth_cancelled") {
        terminalRef.current = true;
        promptRef.current = undefined;
        setPrompt(undefined);
        setActionError(undefined);
        setPendingPromptId(undefined);
        setPhase("cancelled");
      } else if (record.type === "auth_failed") {
        terminalRef.current = true;
        promptRef.current = undefined;
        setPrompt(undefined);
        setActionError(undefined);
        setPendingPromptId(undefined);
        setTerminalMessage(record.message);
        setPhase("failed");
      }
    };

    void adapter.connect(receive).then(
      (connected) => {
        if (!active) {
          void connected.cancel().catch(() => undefined);
          connected.disconnect();
          return;
        }
        session = connected;
        sessionRef.current = connected;
        setPhase((current) => (current === "connecting" ? "active" : current));
      },
      () => {
        if (!active) return;
        terminalRef.current = true;
        setTerminalMessage("Più couldn’t start Codex sign-in. Try again.");
        setPhase("failed");
      },
    );

    return () => {
      active = false;
      if (session && !terminalRef.current) void session.cancel().catch(() => undefined);
      session?.disconnect();
      if (sessionRef.current === session) sessionRef.current = undefined;
    };
  }, [adapter, attempt]);

  const answer = useCallback(
    async (promptId: string, value: string) => {
      const session = sessionRef.current;
      if (!session || pendingPromptId) return;
      setPendingPromptId(promptId);
      setActionError(undefined);
      try {
        await session.answer(promptId, value);
        if (promptRef.current?.id === promptId) {
          promptRef.current = undefined;
          setPrompt(undefined);
        }
      } catch {
        if (promptRef.current?.id === promptId) {
          setActionError("Più couldn’t answer that sign-in prompt. Try again.");
        }
      } finally {
        setPendingPromptId((current) => (current === promptId ? undefined : current));
      }
    },
    [pendingPromptId],
  );

  const submitPrompt = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const activePrompt = promptRef.current;
      if (!activePrompt) return;
      const value = new FormData(event.currentTarget).get("answer");
      if (typeof value !== "string" || value.length === 0) return;
      void answer(activePrompt.id, value);
    },
    [answer],
  );

  const cancel = useCallback(async () => {
    const session = sessionRef.current;
    if (!session || phase === "cancelling") return;
    setPhase("cancelling");
    setActionError(undefined);
    try {
      await session.cancel();
    } catch {
      if (!terminalRef.current) {
        setPhase("active");
        setActionError("Più couldn’t cancel sign-in. Try again.");
      }
    }
  }, [phase]);

  const retry = useCallback(() => {
    setPrompt(undefined);
    setNotifications([]);
    setActionError(undefined);
    setPendingPromptId(undefined);
    setCopiedCode(undefined);
    setTerminalMessage(undefined);
    setPhase("connecting");
    setAttempt((current) => current + 1);
  }, []);

  const runSessionAction = useCallback<RunSessionAction>(async (action) => {
    const session = sessionRef.current;
    if (!session) return false;
    setActionError(undefined);
    try {
      await action(session);
      return true;
    } catch {
      setActionError("That action didn’t work. Try again.");
      return false;
    }
  }, []);

  return (
    <section
      aria-busy={phase === "connecting" || phase === "cancelling"}
      aria-labelledby="codex-sign-in-title"
      className="codex-auth"
    >
      <header className="codex-auth__header">
        <span aria-hidden="true" className="codex-auth__mark">
          <KeyRoundIcon />
        </span>
        <div>
          <h2 id="codex-sign-in-title">Sign in to Codex</h2>
          <p>
            Your browser handles sign-in. Più keeps the resulting account session in macOS Keychain.
          </p>
        </div>
      </header>

      {notifications.length > 0 ? (
        <div aria-label="Sign-in updates" aria-live="polite" className="codex-auth__updates">
          {notifications.map((item) => (
            <NotificationView
              copiedCode={copiedCode}
              item={item}
              key={item.id}
              onCodeCopied={setCopiedCode}
              runSessionAction={runSessionAction}
            />
          ))}
        </div>
      ) : null}

      {prompt?.prompt.type === "select" ? (
        <div aria-label={prompt.prompt.message} className="codex-auth__prompt" role="group">
          <p className="codex-auth__prompt-label">{prompt.prompt.message}</p>
          <div className="codex-auth__choices">
            {prompt.prompt.options.map((option, index) => (
              <Button
                autoFocus={index === 0}
                className="codex-auth__choice"
                disabled={Boolean(pendingPromptId)}
                key={option.id}
                onClick={() => void answer(prompt.id, option.id)}
                type="button"
                variant="outline"
              >
                <span className="codex-auth__choice-copy">
                  <strong>{option.label}</strong>
                  {option.description ? <small>{option.description}</small> : null}
                </span>
                <ChevronRightIcon aria-hidden="true" className="codex-auth__choice-arrow" />
              </Button>
            ))}
          </div>
        </div>
      ) : null}

      {prompt && prompt.prompt.type !== "select" ? (
        <form className="codex-auth__prompt-form" key={prompt.id} onSubmit={submitPrompt}>
          <div className="codex-auth__field">
            <label htmlFor={fieldId}>{prompt.prompt.message}</label>
            <Input
              autoComplete="off"
              autoFocus
              id={fieldId}
              name="answer"
              placeholder={prompt.prompt.placeholder}
              required
              type={prompt.prompt.type === "secret" ? "password" : "text"}
            />
          </div>
          <Button disabled={pendingPromptId === prompt.id} type="submit">
            {pendingPromptId === prompt.id
              ? "Submitting"
              : prompt.prompt.type === "manual_code"
                ? "Submit code"
                : "Continue"}
          </Button>
        </form>
      ) : null}

      {phase === "connecting" ? (
        <div className="codex-auth__progress" role="status">
          <AuthSpinner />
          <p>Starting Codex sign-in</p>
        </div>
      ) : null}
      {phase === "complete" ? (
        <div className="codex-auth__terminal" data-kind="complete" role="status">
          <CheckCircle2Icon aria-hidden="true" />
          <div>
            <strong>Codex is connected</strong>
            <p>New cloud chats can use your ChatGPT subscription.</p>
          </div>
        </div>
      ) : null}
      {phase === "cancelled" ? (
        <div className="codex-auth__terminal" role="status">
          <CircleAlertIcon aria-hidden="true" />
          <div>
            <strong>Sign-in cancelled</strong>
            <p>No account changes were made.</p>
          </div>
        </div>
      ) : null}
      {phase === "failed" ? (
        <div className="codex-auth__terminal" data-kind="failed" role="alert">
          <CircleAlertIcon aria-hidden="true" />
          <div>
            <strong>Couldn&apos;t sign in</strong>
            <p>{terminalMessage ?? "Sign-in failed. Try again."}</p>
          </div>
        </div>
      ) : null}
      {actionError ? (
        <p className="codex-auth__error" role="alert">
          {actionError}
        </p>
      ) : null}

      <footer className="codex-auth__footer">
        {phase === "active" || phase === "connecting" || phase === "cancelling" ? (
          <Button
            disabled={phase === "connecting" || phase === "cancelling"}
            onClick={() => void cancel()}
            type="button"
            variant="ghost"
          >
            {phase === "cancelling" ? "Cancelling" : "Cancel sign-in"}
          </Button>
        ) : null}
        {phase === "cancelled" || phase === "failed" ? (
          <Button onClick={retry} type="button">
            Try again
          </Button>
        ) : null}
      </footer>
    </section>
  );
}

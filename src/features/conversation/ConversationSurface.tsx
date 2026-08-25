import {
  ArrowUpIcon,
  CheckIcon,
  ChevronDownIcon,
  CircleAlertIcon,
  KeyRoundIcon,
  LoaderCircleIcon,
  RotateCcwIcon,
  SquareIcon,
  XIcon,
} from "lucide-react";
import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type UIEvent,
} from "react";

import { ProductComposer } from "@/components/ProductComposer";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  conversationErrorMessage,
  conversationRequiresCodexSignIn,
  type ConversationItem,
  type ConversationPhase,
  type ConversationTool,
} from "@/platform/conversations";

import { ConversationStore } from "./conversation-store";
import "./conversation.css";

interface ConversationSurfaceProps {
  draft?: string;
  onDraftChange?: (value: string) => void;
  onRequestCodexSignIn?: () => void;
  onSend?: (text: string) => Promise<void>;
  onStop?: () => Promise<void>;
  recovery?: {
    message: string;
    onRequestCodexSignIn?: () => void;
    onRetry: () => void;
  };
  store?: ConversationStore;
}

const EMPTY_CONVERSATION = new ConversationStore({
  failure: null,
  items: [],
  phase: "stopped",
});

const tokenFormatter = new Intl.NumberFormat("en-US");

function ToolStatusIcon({ status }: { status: ConversationTool["status"] }) {
  if (status === "running") {
    return <LoaderCircleIcon aria-hidden="true" className="conversation-spin" />;
  }
  if (status === "succeeded") return <CheckIcon aria-hidden="true" />;
  return <XIcon aria-hidden="true" />;
}

function ToolDetail({ detail }: { detail: string }) {
  return <pre className="conversation-tool-detail">{detail || "Waiting for tool output"}</pre>;
}

function ToolItem({ item }: { item: ConversationTool }) {
  const [open, setOpen] = useState(false);
  if (item.status === "succeeded") {
    return (
      <Collapsible
        className="conversation-tool"
        data-status={item.status}
        onOpenChange={setOpen}
        open={open}
      >
        <CollapsibleTrigger
          aria-label={`${open ? "Hide" : "Show"} ${item.name} details`}
          render={<Button className="conversation-tool-trigger" type="button" variant="ghost" />}
        >
          <ToolStatusIcon status={item.status} />
          <span>{item.name}</span>
          <span className="conversation-tool-state">Completed</span>
          <ChevronDownIcon aria-hidden="true" className="conversation-disclosure" />
        </CollapsibleTrigger>
        <CollapsibleContent>
          <ToolDetail detail={item.detail} />
        </CollapsibleContent>
      </Collapsible>
    );
  }

  return (
    <section
      aria-label={`${item.name}, ${item.status}`}
      className="conversation-tool"
      data-status={item.status}
    >
      <div className="conversation-tool-heading">
        <ToolStatusIcon status={item.status} />
        <span>{item.name}</span>
        <span className="conversation-tool-state">
          {item.status === "running" ? "Running" : "Failed"}
        </span>
      </div>
      <ToolDetail detail={item.detail} />
    </section>
  );
}

function ReasoningItem({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="conversation-reasoning-row">
      <p className="conversation-message-role">Più</p>
      <Collapsible className="conversation-reasoning" onOpenChange={setOpen} open={open}>
        <CollapsibleTrigger
          render={
            <Button className="conversation-reasoning-trigger" type="button" variant="ghost" />
          }
        >
          <ChevronDownIcon aria-hidden="true" className="conversation-disclosure" />
          {open ? "Hide reasoning" : "Show reasoning"}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <p>{text || "Reasoning is streaming"}</p>
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
}

function TranscriptItemView({ item }: { item: ConversationItem }) {
  if (item.kind === "reasoning") return <ReasoningItem text={item.text} />;
  if (item.kind === "tool") return <ToolItem item={item} />;
  if (item.kind === "usage") {
    const cached = item.cacheReadTokens
      ? ` · ${tokenFormatter.format(item.cacheReadTokens)} cached`
      : "";
    return (
      <p className="conversation-usage">
        {tokenFormatter.format(item.inputTokens)} in · {tokenFormatter.format(item.outputTokens)}{" "}
        out
        {cached}
      </p>
    );
  }
  return (
    <article className="conversation-message" data-role={item.role}>
      <p className="conversation-message-role">{item.role === "user" ? "You" : "Più"}</p>
      <p className="conversation-message-copy">{item.text}</p>
    </article>
  );
}

const TranscriptItem = memo(function TranscriptItem({
  itemId,
  onContentChange,
  store,
}: {
  itemId: string;
  onContentChange: () => void;
  store: ConversationStore;
}) {
  const subscribe = useCallback(
    (listener: () => void) => store.subscribeItem(itemId, listener),
    [itemId, store],
  );
  const getSnapshot = useCallback(() => store.getItem(itemId), [itemId, store]);
  const item = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  useEffect(onContentChange, [item, onContentChange]);
  return item ? <TranscriptItemView item={item} /> : null;
});

function TurnState({ failure, phase }: { failure: string | null; phase: ConversationPhase }) {
  if (phase === "stopped") {
    return (
      <section className="conversation-turn-state">
        <SquareIcon aria-hidden="true" />
        <div>
          <strong>Turn stopped</strong>
          <p>Send another message to continue this chat.</p>
        </div>
      </section>
    );
  }
  if (phase === "failed") {
    return (
      <section className="conversation-turn-state" data-state="failed">
        <CircleAlertIcon aria-hidden="true" />
        <div>
          <strong>Turn failed</strong>
          <p>{failure || "The agent stopped before finishing this turn."}</p>
        </div>
      </section>
    );
  }
  return null;
}

function completedTurnAssistantMessage(store: ConversationStore, itemIds: readonly string[]) {
  let latestUserIndex = -1;
  for (let index = itemIds.length - 1; index >= 0; index -= 1) {
    const item = store.getItem(itemIds[index]);
    if (item?.kind === "message" && item.role === "user") {
      latestUserIndex = index;
      break;
    }
  }
  if (latestUserIndex < 0) return undefined;

  for (let index = itemIds.length - 1; index > latestUserIndex; index -= 1) {
    const item = store.getItem(itemIds[index]);
    if (item?.kind === "message" && item.role === "assistant" && item.text.trim()) {
      return item.text.trim();
    }
  }
  return undefined;
}

function TurnAnnouncement({
  failure,
  itemIds,
  phase,
  store,
}: {
  failure: string | null;
  itemIds: readonly string[];
  phase: ConversationPhase;
  store: ConversationStore;
}) {
  let announcement: string;
  if (phase === "running") {
    announcement = "Più started responding.";
  } else if (phase === "stopped") {
    announcement = "Più stopped responding.";
  } else if (phase === "failed") {
    announcement = `Più failed to respond. ${failure || "The agent stopped before finishing this turn."}`;
  } else {
    const message = completedTurnAssistantMessage(store, itemIds);
    announcement = message ? `Più finished responding. ${message}` : "Più finished responding.";
  }

  return (
    <p aria-atomic="true" aria-live="polite" className="sr-only" role="status">
      {announcement}
    </p>
  );
}

function ConversationComposer({
  active,
  draft,
  onDraftChange,
  onRequestCodexSignIn,
  onSend,
  onStop,
}: {
  active: boolean;
  draft: string;
  onDraftChange: (value: string) => void;
  onRequestCodexSignIn?: () => void;
  onSend?: (text: string) => Promise<void>;
  onStop?: () => Promise<void>;
}) {
  const [error, setError] = useState<{ authenticationRecovery: boolean; message: string }>();
  const [pendingAction, setPendingAction] = useState<"send" | "stop">();

  const send = useCallback(async () => {
    const text = draft.trim();
    if (!text || !onSend || pendingAction) return;
    setPendingAction("send");
    setError(undefined);
    try {
      await onSend(text);
      onDraftChange("");
    } catch (sendError) {
      const authenticationRecovery = conversationRequiresCodexSignIn(sendError);
      setError({
        authenticationRecovery,
        message: conversationErrorMessage(
          sendError,
          "Più couldn’t send that message. Your draft is still here.",
        ),
      });
    } finally {
      setPendingAction(undefined);
    }
  }, [draft, onDraftChange, onSend, pendingAction]);

  const stop = useCallback(async () => {
    if (!onStop || pendingAction) return;
    setPendingAction("stop");
    setError(undefined);
    try {
      await onStop();
    } catch {
      setError({
        authenticationRecovery: false,
        message: "Più couldn’t stop this turn. Try again.",
      });
    } finally {
      setPendingAction(undefined);
    }
  }, [onStop, pendingAction]);

  return (
    <ProductComposer
      actions={
        <>
          {active ? (
            <Button
              aria-label="Stop turn"
              disabled={!onStop || Boolean(pendingAction)}
              onClick={() => void stop()}
              size="icon"
              type="button"
              variant="outline"
            >
              <SquareIcon aria-hidden="true" />
            </Button>
          ) : null}
          <Button
            aria-label={active ? "Steer active turn" : "Send message"}
            disabled={!onSend || !draft.trim() || Boolean(pendingAction)}
            size="icon"
            type="submit"
          >
            {pendingAction === "send" ? (
              <LoaderCircleIcon aria-hidden="true" className="conversation-spin" />
            ) : (
              <ArrowUpIcon aria-hidden="true" />
            )}
          </Button>
        </>
      }
      ariaLabel="Message Più"
      error={
        error
          ? {
              action:
                error.authenticationRecovery && onRequestCodexSignIn ? (
                  <Button onClick={onRequestCodexSignIn} size="sm" type="button" variant="outline">
                    <KeyRoundIcon aria-hidden="true" />
                    Sign in to Codex
                  </Button>
                ) : undefined,
              message: error.message,
            }
          : undefined
      }
      layout="docked"
      onSubmit={() => void send()}
      onValueChange={onDraftChange}
      placeholder={active ? "Steer the active turn" : "Continue the conversation"}
      status={
        <span>
          {active ? "Sends at the next safe point" : onSend ? "⌘↵ to send" : "Reconnect to send"}
        </span>
      }
      submitOnMetaEnter
      value={draft}
    />
  );
}

export function ConversationSurface({
  draft: controlledDraft,
  onDraftChange,
  onRequestCodexSignIn,
  onSend,
  onStop,
  recovery,
  store = EMPTY_CONVERSATION,
}: ConversationSurfaceProps = {}) {
  const [localDraft, setLocalDraft] = useState("");
  const draft = controlledDraft ?? localDraft;
  const updateDraft = onDraftChange ?? setLocalDraft;
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const endRef = useRef<HTMLDivElement>(null);
  const followTranscriptRef = useRef(true);
  const scrollFrameRef = useRef<number | undefined>(undefined);
  const transcriptScrollRef = useRef<HTMLDivElement>(null);
  const followLatestContent = useCallback(() => {
    if (!followTranscriptRef.current) return;
    if (scrollFrameRef.current !== undefined) cancelAnimationFrame(scrollFrameRef.current);
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = undefined;
      endRef.current?.scrollIntoView?.({ block: "end" });
    });
  }, []);
  const trackScrollPosition = useCallback((event: UIEvent<HTMLDivElement>) => {
    const viewport = event.currentTarget;
    const remaining = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
    followTranscriptRef.current = remaining < 48;
  }, []);

  useEffect(
    () => () => {
      if (scrollFrameRef.current !== undefined) cancelAnimationFrame(scrollFrameRef.current);
    },
    [],
  );

  useEffect(() => {
    const viewport = transcriptScrollRef.current;
    if (!viewport || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(followLatestContent);
    observer.observe(viewport);
    return () => observer.disconnect();
  }, [followLatestContent]);

  return (
    <section aria-label="Conversation" className="conversation-surface">
      <TurnAnnouncement
        failure={snapshot.failure}
        itemIds={snapshot.itemIds}
        phase={snapshot.phase}
        store={store}
      />
      <div
        className="conversation-transcript-scroll"
        onScroll={trackScrollPosition}
        ref={transcriptScrollRef}
      >
        <div className="conversation-transcript">
          {recovery ? (
            <section
              aria-label="Couldn't connect to this chat"
              className="conversation-connection conversation-connection-failure conversation-recovery"
              role="alert"
            >
              <CircleAlertIcon aria-hidden="true" />
              <div className="conversation-connection-copy">
                <h2>Couldn’t connect to this chat</h2>
                <p>{recovery.message}</p>
                <p>Your chat and isolated worktree are unchanged.</p>
                <div className="conversation-connection-actions">
                  <Button onClick={recovery.onRetry} type="button">
                    <RotateCcwIcon aria-hidden="true" />
                    Retry
                  </Button>
                  {recovery.onRequestCodexSignIn ? (
                    <Button onClick={recovery.onRequestCodexSignIn} type="button" variant="outline">
                      <KeyRoundIcon aria-hidden="true" />
                      Sign in to Codex
                    </Button>
                  ) : null}
                </div>
              </div>
            </section>
          ) : snapshot.itemIds.length > 0 ? (
            snapshot.itemIds.map((itemId) => (
              <TranscriptItem
                itemId={itemId}
                key={itemId}
                onContentChange={followLatestContent}
                store={store}
              />
            ))
          ) : (
            <div className="conversation-empty">
              <p>No messages yet</p>
              <span>Send a message to continue this chat.</span>
            </div>
          )}
          {recovery ? null : <TurnState failure={snapshot.failure} phase={snapshot.phase} />}
          <div aria-hidden="true" ref={endRef} />
        </div>
      </div>
      <div className="conversation-composer-dock">
        <ConversationComposer
          active={snapshot.phase === "running"}
          draft={draft}
          onDraftChange={updateDraft}
          onRequestCodexSignIn={onRequestCodexSignIn}
          onSend={onSend}
          onStop={onStop}
        />
      </div>
    </section>
  );
}

export default ConversationSurface;

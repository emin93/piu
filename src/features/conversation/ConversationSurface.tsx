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
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  Virtuoso,
  type Components,
  type ContextProp,
  type FollowOutput,
  type ItemProps,
  type ListProps,
  type VirtuosoHandle,
} from "react-virtuoso";

import { ProductComposer } from "@/components/ProductComposer";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  PromptAttachmentButton,
  PromptAttachmentTray,
} from "@/features/attachments/PromptAttachments";
import {
  conversationErrorMessage,
  conversationRequiresCodexSignIn,
  type ConversationItem,
  type ConversationInputAnswer,
  type ConversationPhase,
  type ConversationTool,
} from "@/platform/conversations";
import type { PromptAttachment } from "@/platform/prompt-attachments";

import { ConversationStore } from "./conversation-store";
import { ConversationInputDialog } from "./ConversationInputDialog";
import "./conversation.css";

interface ConversationSurfaceProps {
  attachments?: readonly PromptAttachment[];
  draft?: string;
  onAnswerInput?: (requestId: string, answer: ConversationInputAnswer) => Promise<void>;
  onAttachmentsChange?: (attachments: PromptAttachment[]) => void;
  onDraftChange?: (value: string) => void;
  onRequestCodexSignIn?: () => void;
  onSend?: (text: string, attachments: readonly PromptAttachment[]) => Promise<void>;
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
  inputRequest: null,
  items: [],
  phase: "stopped",
});

const tokenFormatter = new Intl.NumberFormat("en-US");

function ToolStatusIcon({ status }: { status: ConversationTool["status"] }) {
  if (status === "running") {
    return <LoaderCircleIcon aria-hidden="true" className="conversation-spin" />;
  }
  if (status === "succeeded") return <CheckIcon aria-hidden="true" />;
  if (status === "interrupted") return <SquareIcon aria-hidden="true" />;
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
          {item.status === "running"
            ? "Running"
            : item.status === "interrupted"
              ? "Interrupted"
              : "Failed"}
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
    <article
      className="conversation-message"
      data-queued={item.queued || undefined}
      data-role={item.role}
    >
      <p className="conversation-message-role">{item.role === "user" ? "You" : "Più"}</p>
      <div className="conversation-message-body">
        <p className="conversation-message-copy">{item.text}</p>
        {item.queued ? (
          <p className="conversation-message-queue">Queued · next safe point</p>
        ) : null}
      </div>
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
  if (phase === "interrupted") {
    return (
      <section className="conversation-turn-state" data-state="interrupted">
        <SquareIcon aria-hidden="true" />
        <div>
          <strong>Turn interrupted</strong>
          <p>{failure || "The agent stopped before finishing this turn."}</p>
        </div>
      </section>
    );
  }
  return null;
}

interface TranscriptListContext {
  failure: string | null;
  phase: ConversationPhase;
  store: ConversationStore;
}

const TranscriptHeader = () => (
  <div aria-hidden="true" className="conversation-transcript-header" />
);

function TranscriptFooter({ context }: ContextProp<TranscriptListContext>) {
  return (
    <div className="conversation-transcript-footer">
      <TurnState failure={context.failure} phase={context.phase} />
    </div>
  );
}

const TranscriptList = forwardRef<HTMLDivElement, ContextProp<TranscriptListContext> & ListProps>(
  function TranscriptList({ children, context: _context, style, ...props }, ref) {
    void _context;
    return (
      <div {...props} className="conversation-transcript" ref={ref} style={style}>
        {children}
      </div>
    );
  },
);

const TranscriptItemContainer = forwardRef<
  HTMLDivElement,
  ContextProp<TranscriptListContext> & ItemProps<string>
>(function TranscriptItemContainer({ children, context, item: itemId, style, ...props }, ref) {
  const item = context.store.getItem(itemId);
  return (
    <div
      {...props}
      data-transcript-kind={item?.kind}
      data-transcript-role={item?.kind === "message" ? item.role : undefined}
      className="conversation-transcript-item"
      ref={ref}
      style={style}
    >
      {children}
    </div>
  );
});

const TRANSCRIPT_COMPONENTS: Components<string, TranscriptListContext> = {
  Footer: TranscriptFooter,
  Header: TranscriptHeader,
  Item: TranscriptItemContainer,
  List: TranscriptList,
};

const TRANSCRIPT_VIEWPORT_BUFFER = { bottom: 160, top: 240 };
const transcriptItemKey = (_index: number, itemId: string) => itemId;

function transcriptHasActiveInteraction(transcript: HTMLElement | null) {
  if (!transcript) return false;
  const activeElement = document.activeElement;
  if (activeElement && activeElement !== transcript && transcript.contains(activeElement)) {
    return true;
  }

  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) return false;
  return Boolean(
    (selection.anchorNode && transcript.contains(selection.anchorNode)) ||
    (selection.focusNode && transcript.contains(selection.focusNode)),
  );
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
  } else if (phase === "interrupted") {
    announcement = `Più was interrupted. ${failure || "The agent stopped before finishing this turn."}`;
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
  attachments,
  draft,
  onAttachmentsChange,
  onDraftChange,
  onRequestCodexSignIn,
  onSend,
  onStop,
}: {
  active: boolean;
  attachments: readonly PromptAttachment[];
  draft: string;
  onAttachmentsChange: (attachments: PromptAttachment[]) => void;
  onDraftChange: (value: string) => void;
  onRequestCodexSignIn?: () => void;
  onSend?: (text: string, attachments: readonly PromptAttachment[]) => Promise<void>;
  onStop?: () => Promise<void>;
}) {
  const [error, setError] = useState<{ authenticationRecovery: boolean; message: string }>();
  const [pendingAction, setPendingAction] = useState<"send" | "stop">();

  const send = useCallback(async () => {
    const text = draft.trim();
    if ((!text && attachments.length === 0) || !onSend || pendingAction) return;
    setPendingAction("send");
    setError(undefined);
    try {
      await onSend(text, attachments);
      onDraftChange("");
      onAttachmentsChange([]);
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
  }, [attachments, draft, onAttachmentsChange, onDraftChange, onSend, pendingAction]);

  const removeAttachment = useCallback(
    (attachmentId: string) => {
      onAttachmentsChange(attachments.filter((attachment) => attachment.id !== attachmentId));
    },
    [attachments, onAttachmentsChange],
  );

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
      attachments={
        <PromptAttachmentTray
          attachments={attachments}
          disabled={pendingAction === "send"}
          onRemove={removeAttachment}
        />
      }
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
            disabled={
              !onSend || (!draft.trim() && attachments.length === 0) || Boolean(pendingAction)
            }
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
      inputReadOnly={pendingAction === "send"}
      leadingActions={
        <PromptAttachmentButton
          attachments={attachments}
          disabled={!onSend || Boolean(pendingAction)}
          onChange={onAttachmentsChange}
          onError={(message) =>
            setError(message ? { authenticationRecovery: false, message } : undefined)
          }
        />
      }
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
  attachments: controlledAttachments,
  draft: controlledDraft,
  onAnswerInput,
  onAttachmentsChange,
  onDraftChange,
  onRequestCodexSignIn,
  onSend,
  onStop,
  recovery,
  store = EMPTY_CONVERSATION,
}: ConversationSurfaceProps = {}) {
  const [localDraft, setLocalDraft] = useState("");
  const [localAttachments, setLocalAttachments] = useState<PromptAttachment[]>([]);
  const draft = controlledDraft ?? localDraft;
  const attachments = controlledAttachments ?? localAttachments;
  const updateDraft = onDraftChange ?? setLocalDraft;
  const updateAttachments = onAttachmentsChange ?? setLocalAttachments;
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const followTranscriptRef = useRef(true);
  const scrollFrameRef = useRef<number | undefined>(undefined);
  const transcriptRef = useRef<VirtuosoHandle>(null);
  const transcriptScrollerRef = useRef<HTMLElement | null>(null);
  const transcriptContext = useMemo<TranscriptListContext>(
    () => ({ failure: snapshot.failure, phase: snapshot.phase, store }),
    [snapshot.failure, snapshot.phase, store],
  );
  const initialTranscriptPosition = useMemo(
    () => ({ align: "end" as const, index: snapshot.itemIds.length - 1 }),
    [snapshot.itemIds.length],
  );
  const followLatestContent = useCallback(() => {
    if (
      !followTranscriptRef.current ||
      transcriptHasActiveInteraction(transcriptScrollerRef.current)
    ) {
      return;
    }
    if (scrollFrameRef.current !== undefined) cancelAnimationFrame(scrollFrameRef.current);
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = undefined;
      transcriptRef.current?.autoscrollToBottom();
    });
  }, []);
  const trackScrollPosition = useCallback((atBottom: boolean) => {
    followTranscriptRef.current = atBottom;
  }, []);
  const trackTranscriptScroller = useCallback((scroller: HTMLElement | Window | null) => {
    transcriptScrollerRef.current = scroller instanceof HTMLElement ? scroller : null;
  }, []);
  const followAppendedTranscript: FollowOutput = useCallback(
    (atBottom: boolean) =>
      atBottom && !transcriptHasActiveInteraction(transcriptScrollerRef.current) ? "auto" : false,
    [],
  );
  const renderTranscriptItem = useCallback(
    (_index: number, itemId: string) => (
      <TranscriptItem itemId={itemId} onContentChange={followLatestContent} store={store} />
    ),
    [followLatestContent, store],
  );

  useEffect(
    () => () => {
      if (scrollFrameRef.current !== undefined) cancelAnimationFrame(scrollFrameRef.current);
    },
    [],
  );

  const inputRequest = snapshot.inputRequest;

  return (
    <section aria-label="Conversation" className="conversation-surface">
      {inputRequest && onAnswerInput ? (
        <ConversationInputDialog
          key={inputRequest.id}
          onAnswer={(answer) => onAnswerInput(inputRequest.id, answer)}
          request={inputRequest}
        />
      ) : null}
      <TurnAnnouncement
        failure={snapshot.failure}
        itemIds={snapshot.itemIds}
        phase={snapshot.phase}
        store={store}
      />
      {recovery || snapshot.itemIds.length === 0 ? (
        <div className="conversation-transcript-scroll">
          <div className="conversation-static-transcript">
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
                      <Button
                        onClick={recovery.onRequestCodexSignIn}
                        type="button"
                        variant="outline"
                      >
                        <KeyRoundIcon aria-hidden="true" />
                        Sign in to Codex
                      </Button>
                    ) : null}
                  </div>
                </div>
              </section>
            ) : (
              <div className="conversation-empty">
                <p>No messages yet</p>
                <span>Send a message to continue this chat.</span>
              </div>
            )}
          </div>
        </div>
      ) : (
        <Virtuoso
          alignToBottom
          aria-label="Conversation transcript"
          atBottomStateChange={trackScrollPosition}
          atBottomThreshold={48}
          className="conversation-transcript-scroll"
          components={TRANSCRIPT_COMPONENTS}
          computeItemKey={transcriptItemKey}
          context={transcriptContext}
          data={snapshot.itemIds}
          defaultItemHeight={84}
          followOutput={followAppendedTranscript}
          increaseViewportBy={TRANSCRIPT_VIEWPORT_BUFFER}
          initialTopMostItemIndex={initialTranscriptPosition}
          itemContent={renderTranscriptItem}
          ref={transcriptRef}
          role="region"
          scrollerRef={trackTranscriptScroller}
        />
      )}
      <div className="conversation-composer-dock">
        <ConversationComposer
          active={snapshot.phase === "running"}
          attachments={attachments}
          draft={draft}
          onAttachmentsChange={updateAttachments}
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

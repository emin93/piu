import { LoaderCircleIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import {
  type ConversationAdapter,
  conversationErrorMessage,
  conversationRequiresCodexSignIn,
} from "@/platform/conversations";
import { type ModelControlsAdapter, tauriModelControlsAdapter } from "@/platform/model-controls";
import type { PromptAttachment } from "@/platform/prompt-attachments";

import {
  createChatConversationSession,
  readCachedChatConversationSession,
  rememberCachedChatConversationSession,
} from "./chat-conversation-session-cache";
import { ConversationSurface, type TranscriptViewState } from "./ConversationSurface";
import { ConversationController } from "./conversation-controller";

interface ChatConversationPanelProps {
  adapter: ConversationAdapter;
  cacheOwner?: object;
  chatId: string;
  initialTranscriptState?: TranscriptViewState;
  modelControlsAdapter?: ModelControlsAdapter;
  onRequestCodexSignIn: () => void;
  onTranscriptStateChange?: (state: TranscriptViewState) => void;
  revision?: number;
}

type ConnectionState =
  | { controller: ConversationController; phase: "connecting" }
  | { controller: ConversationController; phase: "connected" }
  | { controller: ConversationController; error: unknown; phase: "failed" };

function failureMessage(error: unknown) {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  return conversationErrorMessage(error, "Più couldn’t start the agent runtime.");
}

function ConnectedChatConversationPanel({
  adapter,
  cacheOwner,
  chatId,
  initialTranscriptState,
  modelControlsAdapter,
  onRequestCodexSignIn,
  onTranscriptStateChange,
  revision = 0,
}: ChatConversationPanelProps) {
  const effectiveModelControlsAdapter = modelControlsAdapter ?? tauriModelControlsAdapter;
  const cachedSession = cacheOwner
    ? readCachedChatConversationSession(cacheOwner, chatId)
    : undefined;
  const session = useMemo(
    () =>
      cachedSession ??
      createChatConversationSession(adapter, chatId, effectiveModelControlsAdapter),
    [adapter, cachedSession, chatId, effectiveModelControlsAdapter],
  );
  const { composer, controller, modelControls } = session;
  const [connection, setConnection] = useState<ConnectionState>(() => ({
    controller,
    phase: "connecting",
  }));
  const [attempt, setAttempt] = useState(0);
  const [draft, setDraftState] = useState(composer.draft);
  const [attachments, setAttachmentsState] = useState<PromptAttachment[]>(composer.attachments);
  const activeConnection: ConnectionState =
    connection.controller === controller ? connection : { controller, phase: "connecting" };
  const showCachedConversation = activeConnection.phase === "connecting" && controller.hasSnapshot;

  useEffect(() => {
    if (cacheOwner) rememberCachedChatConversationSession(cacheOwner, chatId, session);
  }, [cacheOwner, chatId, session]);

  useEffect(() => {
    let active = true;
    void controller.connect().then(
      () => {
        if (active) setConnection({ controller, phase: "connected" });
      },
      (error: unknown) => {
        if (active) setConnection({ controller, error, phase: "failed" });
      },
    );
    return () => {
      active = false;
      controller.dispose();
    };
  }, [attempt, controller, revision]);

  useEffect(() => {
    void modelControls.load();
  }, [attempt, modelControls, revision]);

  useEffect(() => () => modelControls.dispose(), [modelControls]);

  const send = useCallback(
    (text: string, selectedAttachments: readonly PromptAttachment[]) =>
      controller.send(text, selectedAttachments),
    [controller],
  );
  const answerInput = useCallback(
    (requestId: string, answer: Parameters<ConversationController["answerInput"]>[1]) =>
      controller.answerInput(requestId, answer),
    [controller],
  );
  const stop = useCallback(() => controller.stop(), [controller]);
  const setDraft = useCallback(
    (value: string) => {
      composer.rememberDraft(value);
      setDraftState(value);
    },
    [composer],
  );
  const setAttachments = useCallback(
    (value: PromptAttachment[]) => {
      composer.rememberAttachments(value);
      setAttachmentsState(value);
    },
    [composer],
  );
  const retry = useCallback(() => {
    setConnection({ controller, phase: "connecting" });
    setAttempt((current) => current + 1);
  }, [controller]);

  if (activeConnection.phase === "connected" || showCachedConversation) {
    const connected = activeConnection.phase === "connected";
    return (
      <ConversationSurface
        attachments={attachments}
        draft={draft}
        initialTranscriptState={initialTranscriptState}
        onAnswerInput={connected ? answerInput : undefined}
        onAttachmentsChange={setAttachments}
        onDraftChange={setDraft}
        onRequestCodexSignIn={onRequestCodexSignIn}
        onSend={connected ? send : undefined}
        onStop={connected ? stop : undefined}
        onTranscriptStateChange={onTranscriptStateChange}
        modelControls={modelControls}
        store={controller.store}
      />
    );
  }

  if (activeConnection.phase === "failed") {
    const requestCodexSignIn = conversationRequiresCodexSignIn(activeConnection.error)
      ? onRequestCodexSignIn
      : undefined;
    return (
      <ConversationSurface
        attachments={attachments}
        draft={draft}
        initialTranscriptState={initialTranscriptState}
        onAnswerInput={answerInput}
        onAttachmentsChange={setAttachments}
        onDraftChange={setDraft}
        onRequestCodexSignIn={requestCodexSignIn}
        onTranscriptStateChange={onTranscriptStateChange}
        recovery={{
          message: failureMessage(activeConnection.error),
          onRequestCodexSignIn: requestCodexSignIn,
          onRetry: retry,
        }}
        store={controller.store}
      />
    );
  }

  return (
    <div aria-busy="true" className="conversation-connection" role="status">
      <LoaderCircleIcon aria-hidden="true" className="conversation-spin" />
      <span>Connecting to chat</span>
    </div>
  );
}

export function ChatConversationPanel(props: ChatConversationPanelProps) {
  return <ConnectedChatConversationPanel key={props.chatId} {...props} />;
}

export default ChatConversationPanel;

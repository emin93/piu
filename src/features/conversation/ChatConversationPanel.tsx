import { LoaderCircleIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import {
  type ConversationAdapter,
  conversationErrorMessage,
  conversationRequiresCodexSignIn,
} from "@/platform/conversations";
import type { PromptAttachment } from "@/platform/prompt-attachments";

import { ConversationSurface } from "./ConversationSurface";
import { ConversationController } from "./conversation-controller";

interface ChatConversationPanelProps {
  adapter: ConversationAdapter;
  chatId: string;
  onRequestCodexSignIn: () => void;
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
  chatId,
  onRequestCodexSignIn,
  revision = 0,
}: ChatConversationPanelProps) {
  const controller = useMemo(() => new ConversationController(chatId, adapter), [adapter, chatId]);
  const [connection, setConnection] = useState<ConnectionState>(() => ({
    controller,
    phase: "connecting",
  }));
  const [attempt, setAttempt] = useState(0);
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<PromptAttachment[]>([]);
  const activeConnection: ConnectionState =
    connection.controller === controller ? connection : { controller, phase: "connecting" };

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
  const retry = useCallback(() => {
    setConnection({ controller, phase: "connecting" });
    setAttempt((current) => current + 1);
  }, [controller]);

  if (activeConnection.phase === "connected") {
    return (
      <ConversationSurface
        attachments={attachments}
        draft={draft}
        onAnswerInput={answerInput}
        onAttachmentsChange={setAttachments}
        onDraftChange={setDraft}
        onRequestCodexSignIn={onRequestCodexSignIn}
        onSend={send}
        onStop={stop}
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
        onAnswerInput={answerInput}
        onAttachmentsChange={setAttachments}
        onDraftChange={setDraft}
        onRequestCodexSignIn={requestCodexSignIn}
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

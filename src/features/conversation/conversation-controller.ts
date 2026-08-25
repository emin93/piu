import type {
  ConversationAdapter,
  ConversationEvent,
  ConversationInputAnswer,
} from "@/platform/conversations";
import type { PromptAttachment } from "@/platform/prompt-attachments";

import { ConversationStore } from "./conversation-store";

const STOPPED_CONVERSATION = {
  failure: null,
  inputRequest: null,
  items: [],
  phase: "stopped",
  revision: 0,
} as const;

interface PendingConversationEvent {
  event: ConversationEvent;
  revision?: number;
}

export class ConversationController {
  readonly #adapter: ConversationAdapter;
  readonly #chatId: string;
  #disconnect: (() => void) | undefined;
  #generation = 0;
  readonly store = new ConversationStore(STOPPED_CONVERSATION);

  constructor(chatId: string, adapter: ConversationAdapter) {
    this.#adapter = adapter;
    this.#chatId = chatId;
  }

  async connect() {
    const generation = ++this.#generation;
    this.#disconnect?.();
    this.#disconnect = undefined;
    const pendingEvents: PendingConversationEvent[] = [];
    let connected = false;
    const connection = await this.#adapter.connect(this.#chatId, (event, revision) => {
      if (generation !== this.#generation) return;
      if (connected) this.store.apply(event);
      else pendingEvents.push({ event, revision });
    });

    if (generation !== this.#generation) {
      connection.disconnect();
      return;
    }
    this.#disconnect = connection.disconnect;
    this.store.replace(connection.snapshot);
    connected = true;
    for (const pending of pendingEvents) {
      if (
        pending.revision === undefined ||
        pending.revision > (connection.snapshot.revision ?? 0)
      ) {
        this.store.apply(pending.event);
      }
    }
  }

  send(text: string, attachments: readonly PromptAttachment[] = []) {
    const trimmedText = text.trim();
    if (!trimmedText && attachments.length === 0) return Promise.resolve();
    return this.#adapter.prompt({
      attachments,
      chatId: this.#chatId,
      streamingBehavior: "steer",
      text: trimmedText,
    });
  }

  stop() {
    return this.#adapter.stop(this.#chatId);
  }

  answerInput(requestId: string, answer: ConversationInputAnswer) {
    return this.#adapter.answerInput(this.#chatId, requestId, answer);
  }

  dispose() {
    this.#generation += 1;
    this.#disconnect?.();
    this.#disconnect = undefined;
  }
}

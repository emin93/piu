import type { ConversationAdapter, ConversationEvent } from "@/platform/conversations";

import { ConversationStore } from "./conversation-store";

const STOPPED_CONVERSATION = {
  failure: null,
  items: [],
  phase: "stopped",
} as const;

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
    const pendingEvents: ConversationEvent[] = [];
    let connected = false;
    const connection = await this.#adapter.connect(this.#chatId, (event) => {
      if (generation !== this.#generation) return;
      if (connected) this.store.apply(event);
      else pendingEvents.push(event);
    });

    if (generation !== this.#generation) {
      connection.disconnect();
      return;
    }
    this.#disconnect = connection.disconnect;
    this.store.replace(connection.snapshot);
    connected = true;
    for (const event of pendingEvents) this.store.apply(event);
  }

  send(text: string) {
    const trimmedText = text.trim();
    if (!trimmedText) return Promise.resolve();
    return this.#adapter.prompt({
      chatId: this.#chatId,
      streamingBehavior: "steer",
      text: trimmedText,
    });
  }

  stop() {
    return this.#adapter.stop(this.#chatId);
  }

  dispose() {
    this.#generation += 1;
    this.#disconnect?.();
    this.#disconnect = undefined;
  }
}

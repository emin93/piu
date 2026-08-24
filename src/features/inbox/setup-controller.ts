import type { ChatSetupChangedEvent, ChatSetupSummary } from "@/platform/chat-workspaces";
import type { InboxSnapshot } from "@/platform/project-inbox";

type SetupListener = () => void;

const UNKNOWN_SETUP: ChatSetupSummary = {
  phase: "pending",
  failure: null,
  exitCode: null,
  signal: null,
  attempt: 0,
  log: "",
};

const phaseProgress = {
  pending: 0,
  running: 1,
  notRequired: 2,
  succeeded: 2,
  failed: 2,
  cancelled: 2,
} satisfies Record<ChatSetupSummary["phase"], number>;

function incomingIsStale(current: ChatSetupSummary, incoming: ChatSetupSummary) {
  if (incoming.attempt !== current.attempt) return incoming.attempt < current.attempt;
  if (phaseProgress[incoming.phase] !== phaseProgress[current.phase]) {
    return phaseProgress[incoming.phase] < phaseProgress[current.phase];
  }
  return incoming.log.length < current.log.length;
}

export class ChatSetupController {
  readonly #entries = new Map<string, ChatSetupSummary>();
  readonly #listeners = new Map<string, Set<SetupListener>>();

  get = (chatId: string) => this.#entries.get(chatId) ?? UNKNOWN_SETUP;

  subscribe = (chatId: string, listener: SetupListener) => {
    const listeners = this.#listeners.get(chatId) ?? new Set<SetupListener>();
    listeners.add(listener);
    this.#listeners.set(chatId, listeners);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) this.#listeners.delete(chatId);
    };
  };

  reconcile(snapshot: InboxSnapshot) {
    const chatIds = new Set(snapshot.chats.map(({ id }) => id));
    for (const chat of snapshot.chats) this.#store(chat.id, chat.setup);
    for (const chatId of this.#entries.keys()) {
      if (!chatIds.has(chatId)) this.#entries.delete(chatId);
    }
  }

  apply(event: ChatSetupChangedEvent) {
    this.#store(event.chatId, event.setup);
  }

  #store(chatId: string, incoming: ChatSetupSummary) {
    const current = this.#entries.get(chatId);
    if (current === incoming || (current && incomingIsStale(current, incoming))) return;
    if (
      current &&
      current.phase === incoming.phase &&
      current.failure === incoming.failure &&
      current.exitCode === incoming.exitCode &&
      current.signal === incoming.signal &&
      current.attempt === incoming.attempt &&
      current.log === incoming.log
    ) {
      return;
    }
    this.#entries.set(chatId, incoming);
    for (const listener of this.#listeners.get(chatId) ?? []) listener();
  }
}

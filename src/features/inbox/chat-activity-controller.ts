type Listener = () => void;

export type ChatActivityPhase =
  "idle" | "running" | "needs-input" | "finished" | "failed" | "interrupted";

export interface ChatActivitySnapshot {
  readonly phase: ChatActivityPhase;
  readonly unread: boolean;
}

export type ChatActivityEvent =
  | { readonly type: "turn-started" }
  | { readonly type: "needs-input" }
  | { readonly type: "input-resolved" }
  | { readonly type: "turn-completed" }
  | { readonly type: "turn-failed" }
  | { readonly type: "turn-interrupted" }
  | { readonly type: "turn-stopped" };

export interface ChatActivityStore {
  readonly getSnapshot: () => ChatActivitySnapshot;
  readonly subscribe: (listener: Listener) => () => void;
}

const IDLE_ACTIVITY: ChatActivitySnapshot = Object.freeze({ phase: "idle", unread: false });
const NO_CHAT_IDS: readonly string[] = Object.freeze([]);

export class ChatActivityController {
  readonly #listeners = new Map<string, Set<Listener>>();
  readonly #snapshots = new Map<string, ChatActivitySnapshot>();
  readonly #stores = new Map<string, ChatActivityStore>();
  #knownChatIds = NO_CHAT_IDS;
  #knownChats = new Set<string>();
  #selectedChatId: string | null = null;

  getKnownChatIds = () => this.#knownChatIds;

  chat(chatId: string): ChatActivityStore {
    const existing = this.#stores.get(chatId);
    if (existing) return existing;
    const store: ChatActivityStore = Object.freeze({
      getSnapshot: () => this.#snapshot(chatId),
      subscribe: (listener: Listener) => this.#subscribe(chatId, listener),
    });
    this.#stores.set(chatId, store);
    return store;
  }

  reconcile(chatIds: readonly string[]) {
    const incoming = new Set(chatIds);
    const nextChatIds = this.#knownChatIds.filter((chatId) => incoming.has(chatId));
    const retained = new Set(nextChatIds);
    for (const chatId of chatIds) {
      if (retained.has(chatId)) continue;
      retained.add(chatId);
      nextChatIds.push(chatId);
    }
    if (
      nextChatIds.length === this.#knownChatIds.length &&
      nextChatIds.every((chatId, index) => chatId === this.#knownChatIds[index])
    ) {
      return;
    }

    for (const chatId of this.#knownChatIds) {
      if (retained.has(chatId)) continue;
      const previous = this.#snapshot(chatId);
      this.#snapshots.delete(chatId);
      if (previous !== IDLE_ACTIVITY) this.#notify(chatId);
    }
    this.#knownChats = retained;
    this.#knownChatIds = Object.freeze(nextChatIds);
    if (this.#selectedChatId && !retained.has(this.#selectedChatId)) {
      this.#selectedChatId = null;
    }
  }

  apply(chatId: string, event: ChatActivityEvent) {
    if (!this.#knownChats.has(chatId)) return;
    const current = this.#snapshot(chatId);
    const background = this.#selectedChatId !== chatId;
    let phase: ChatActivityPhase;
    let unread = current.unread;
    switch (event.type) {
      case "turn-started":
        phase = "running";
        break;
      case "needs-input":
        phase = "needs-input";
        unread ||= background;
        break;
      case "input-resolved":
        phase = "running";
        break;
      case "turn-completed":
        phase = "finished";
        unread ||= background;
        break;
      case "turn-failed":
        phase = "failed";
        unread ||= background;
        break;
      case "turn-interrupted":
        phase = "interrupted";
        unread ||= background;
        break;
      case "turn-stopped":
        phase = "idle";
        unread = false;
        break;
    }
    this.#replace(chatId, phase, unread);
  }

  select(chatId: string | null) {
    this.#selectedChatId = chatId;
    if (!chatId || !this.#knownChats.has(chatId)) return;
    const current = this.#snapshot(chatId);
    if (current.unread) this.#replace(chatId, current.phase, false);
  }

  #notify(chatId: string) {
    for (const listener of this.#listeners.get(chatId) ?? []) listener();
  }

  #replace(chatId: string, phase: ChatActivityPhase, unread: boolean) {
    const current = this.#snapshot(chatId);
    if (current.phase === phase && current.unread === unread) return;
    this.#snapshots.set(chatId, Object.freeze({ phase, unread }));
    this.#notify(chatId);
  }

  #snapshot(chatId: string) {
    return this.#snapshots.get(chatId) ?? IDLE_ACTIVITY;
  }

  #subscribe(chatId: string, listener: Listener) {
    const listeners = this.#listeners.get(chatId) ?? new Set<Listener>();
    listeners.add(listener);
    this.#listeners.set(chatId, listeners);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) this.#listeners.delete(chatId);
    };
  }
}

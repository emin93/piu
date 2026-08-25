import type {
  ConversationEvent,
  ConversationItem,
  ConversationSnapshot,
} from "@/platform/conversations";

type Listener = () => void;

export interface ConversationStoreSnapshot {
  failure: string | null;
  itemIds: readonly string[];
  phase: ConversationSnapshot["phase"];
}

export class ConversationStore {
  readonly #items = new Map<string, ConversationItem>();
  readonly #itemListeners = new Map<string, Set<Listener>>();
  readonly #listeners = new Set<Listener>();
  #snapshot: ConversationStoreSnapshot;

  constructor(snapshot: ConversationSnapshot) {
    for (const item of snapshot.items) this.#items.set(item.id, item);
    this.#snapshot = {
      failure: snapshot.failure,
      itemIds: snapshot.items.map(({ id }) => id),
      phase: snapshot.phase,
    };
  }

  getSnapshot = () => this.#snapshot;

  getItem = (itemId: string) => this.#items.get(itemId);

  subscribe = (listener: Listener) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  subscribeItem(itemId: string, listener: Listener) {
    const listeners = this.#itemListeners.get(itemId) ?? new Set<Listener>();
    listeners.add(listener);
    this.#itemListeners.set(itemId, listeners);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) this.#itemListeners.delete(itemId);
    };
  }

  replace(snapshot: ConversationSnapshot) {
    const previousIds = new Set(this.#snapshot.itemIds);
    const nextIds = new Set(snapshot.items.map(({ id }) => id));
    const changedItemIds = new Set<string>();

    for (const item of snapshot.items) {
      if (this.#items.get(item.id) !== item) changedItemIds.add(item.id);
      this.#items.set(item.id, item);
    }
    for (const itemId of previousIds) {
      if (nextIds.has(itemId)) continue;
      this.#items.delete(itemId);
      changedItemIds.add(itemId);
    }

    this.#snapshot = {
      failure: snapshot.failure,
      itemIds: snapshot.items.map(({ id }) => id),
      phase: snapshot.phase,
    };
    for (const itemId of changedItemIds) {
      for (const listener of this.#itemListeners.get(itemId) ?? []) listener();
    }
    for (const listener of this.#listeners) listener();
  }

  apply(event: ConversationEvent) {
    if (event.type === "item-added") {
      if (this.#items.has(event.item.id)) return;
      this.#items.set(event.item.id, event.item);
      this.#snapshot = {
        ...this.#snapshot,
        itemIds: [...this.#snapshot.itemIds, event.item.id],
      };
      for (const listener of this.#listeners) listener();
      return;
    }

    if (
      event.type === "turn-completed" ||
      event.type === "turn-failed" ||
      event.type === "turn-started" ||
      event.type === "turn-stopped"
    ) {
      const phase =
        event.type === "turn-completed"
          ? "idle"
          : event.type === "turn-failed"
            ? "failed"
            : event.type === "turn-started"
              ? "running"
              : "stopped";
      const failure = event.type === "turn-failed" ? event.message : null;
      if (this.#snapshot.phase === phase && this.#snapshot.failure === failure) return;
      this.#snapshot = { ...this.#snapshot, failure, phase };
      for (const listener of this.#listeners) listener();
      return;
    }

    const item = this.#items.get(event.itemId);
    if (event.type === "text-delta") {
      if (!item || item.kind !== "message" || !event.delta) return;
      this.#items.set(event.itemId, { ...item, text: item.text + event.delta });
    } else if (event.type === "reasoning-delta") {
      if (!item || item.kind !== "reasoning" || !event.delta) return;
      this.#items.set(event.itemId, { ...item, text: item.text + event.delta });
    } else if (event.type === "usage-update") {
      if (!item || item.kind !== "usage") return;
      if (
        item.inputTokens === event.inputTokens &&
        item.outputTokens === event.outputTokens &&
        item.cacheReadTokens === event.cacheReadTokens
      ) {
        return;
      }
      this.#items.set(event.itemId, {
        ...item,
        cacheReadTokens: event.cacheReadTokens,
        inputTokens: event.inputTokens,
        outputTokens: event.outputTokens,
      });
    } else {
      if (!item || item.kind !== "tool") return;
      if (item.status === event.status && item.detail === event.detail) return;
      this.#items.set(event.itemId, {
        ...item,
        detail: event.detail,
        status: event.status,
      });
    }
    for (const listener of this.#itemListeners.get(event.itemId) ?? []) listener();
  }
}

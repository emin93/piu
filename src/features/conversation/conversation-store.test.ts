import { expect, test, vi } from "vitest";

import type { ConversationSnapshot } from "@/platform/conversations";

import { ConversationStore } from "./conversation-store";

const snapshot: ConversationSnapshot = {
  phase: "running",
  items: [
    { id: "user-1", kind: "message", role: "user", text: "Inspect the build." },
    { id: "assistant-1", kind: "message", role: "assistant", text: "I am " },
  ],
  failure: null,
};

test("a streamed text delta updates only its transcript item", () => {
  const store = new ConversationStore(snapshot);
  const shellListener = vi.fn();
  const userListener = vi.fn();
  const assistantListener = vi.fn();

  store.subscribe(shellListener);
  store.subscribeItem("user-1", userListener);
  store.subscribeItem("assistant-1", assistantListener);
  const shellBefore = store.getSnapshot();

  store.apply({ type: "text-delta", itemId: "assistant-1", delta: "checking it now." });

  expect(store.getItem("assistant-1")).toMatchObject({ text: "I am checking it now." });
  expect(store.getSnapshot()).toBe(shellBefore);
  expect(shellListener).not.toHaveBeenCalled();
  expect(userListener).not.toHaveBeenCalled();
  expect(assistantListener).toHaveBeenCalledOnce();
});

test("tool progress stays local while turn completion updates the shell", () => {
  const store = new ConversationStore({
    ...snapshot,
    items: [
      ...snapshot.items,
      {
        id: "tool-1",
        kind: "tool",
        name: "Read files",
        status: "running",
        detail: "Reading src",
      },
    ],
  });
  const shellListener = vi.fn();
  const toolListener = vi.fn();
  store.subscribe(shellListener);
  store.subscribeItem("tool-1", toolListener);

  store.apply({
    type: "tool-update",
    itemId: "tool-1",
    status: "succeeded",
    detail: "Read 12 files",
  });

  expect(store.getItem("tool-1")).toMatchObject({
    status: "succeeded",
    detail: "Read 12 files",
  });
  expect(toolListener).toHaveBeenCalledOnce();
  expect(shellListener).not.toHaveBeenCalled();

  store.apply({ type: "turn-completed" });

  expect(store.getSnapshot()).toMatchObject({ phase: "idle", failure: null });
  expect(shellListener).toHaveBeenCalledOnce();
});

test("reasoning and usage stream through narrow item subscriptions", () => {
  const store = new ConversationStore(snapshot);
  const shellListener = vi.fn();
  const reasoningListener = vi.fn();
  const usageListener = vi.fn();
  store.subscribe(shellListener);

  store.apply({
    type: "item-added",
    item: { id: "reasoning-1", kind: "reasoning", text: "Checking " },
  });
  store.apply({
    type: "item-added",
    item: {
      id: "usage-1",
      kind: "usage",
      inputTokens: 1200,
      outputTokens: 40,
      cacheReadTokens: 800,
    },
  });
  store.subscribeItem("reasoning-1", reasoningListener);
  store.subscribeItem("usage-1", usageListener);

  store.apply({ type: "reasoning-delta", itemId: "reasoning-1", delta: "the manifest." });
  store.apply({
    type: "usage-update",
    itemId: "usage-1",
    inputTokens: 1200,
    outputTokens: 76,
    cacheReadTokens: 800,
  });

  expect(store.getSnapshot().itemIds).toEqual(["user-1", "assistant-1", "reasoning-1", "usage-1"]);
  expect(store.getItem("reasoning-1")).toMatchObject({ text: "Checking the manifest." });
  expect(store.getItem("usage-1")).toMatchObject({ outputTokens: 76 });
  expect(shellListener).toHaveBeenCalledTimes(2);
  expect(reasoningListener).toHaveBeenCalledOnce();
  expect(usageListener).toHaveBeenCalledOnce();
});

test("turn lifecycle events keep prior transcript entries intact", () => {
  const store = new ConversationStore(snapshot);

  store.apply({ type: "turn-stopped" });
  expect(store.getSnapshot()).toMatchObject({ phase: "stopped", failure: null });

  store.apply({ type: "turn-started" });
  store.apply({ type: "turn-failed", message: "The model connection closed." });

  expect(store.getSnapshot()).toMatchObject({
    phase: "failed",
    failure: "The model connection closed.",
    itemIds: ["user-1", "assistant-1"],
  });
  expect(store.getItem("user-1")).toMatchObject({ text: "Inspect the build." });
});

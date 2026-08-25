import { expect, test, vi } from "vitest";

import { ChatActivityController } from "./chat-activity-controller";

test("reconciliation preserves retained state and resets a removed chat", () => {
  const controller = new ChatActivityController();
  controller.reconcile(["chat-a", "chat-b"]);
  const chatAIdle = controller.chat("chat-a").getSnapshot();

  controller.reconcile(["chat-b", "chat-a", "chat-c"]);
  expect(controller.chat("chat-a").getSnapshot()).toBe(chatAIdle);

  controller.apply("chat-a", { type: "turn-started" });
  expect(controller.chat("chat-a").getSnapshot()).toEqual({ phase: "running", unread: false });
  controller.reconcile(["chat-b", "chat-c"]);
  expect(controller.chat("chat-a").getSnapshot()).toEqual({ phase: "idle", unread: false });
});

test("background completion, failure, interruption, and input become unread until selected", () => {
  const controller = new ChatActivityController();
  controller.reconcile(["chat-a", "chat-b", "chat-c"]);
  controller.select("chat-a");

  controller.apply("chat-a", { type: "turn-started" });
  controller.apply("chat-a", { type: "turn-completed" });
  expect(controller.chat("chat-a").getSnapshot()).toEqual({ phase: "finished", unread: false });

  controller.apply("chat-b", { type: "turn-started" });
  controller.apply("chat-b", { type: "needs-input" });
  expect(controller.chat("chat-b").getSnapshot()).toEqual({ phase: "needs-input", unread: true });
  controller.select("chat-b");
  expect(controller.chat("chat-b").getSnapshot()).toEqual({ phase: "needs-input", unread: false });
  controller.apply("chat-b", { type: "input-resolved" });
  expect(controller.chat("chat-b").getSnapshot()).toEqual({ phase: "running", unread: false });

  controller.apply("chat-c", { type: "turn-failed" });
  expect(controller.chat("chat-c").getSnapshot()).toEqual({ phase: "failed", unread: true });
  controller.select("chat-c");
  controller.apply("chat-a", { type: "turn-interrupted" });
  expect(controller.chat("chat-a").getSnapshot()).toEqual({
    phase: "interrupted",
    unread: true,
  });
});

test("a user-stopped turn returns to idle without creating unread activity", () => {
  const controller = new ChatActivityController();
  controller.reconcile(["chat-a"]);

  controller.apply("chat-a", { type: "turn-started" });
  controller.apply("chat-a", { type: "turn-stopped" });

  expect(controller.chat("chat-a").getSnapshot()).toEqual({ phase: "idle", unread: false });
});

test("per-chat subscriptions are isolated and repeated lifecycle events are idempotent", () => {
  const controller = new ChatActivityController();
  controller.reconcile(["chat-a", "chat-b"]);
  const chatAListener = vi.fn();
  const chatBListener = vi.fn();
  const unsubscribeA = controller.chat("chat-a").subscribe(chatAListener);
  controller.chat("chat-b").subscribe(chatBListener);

  controller.apply("chat-a", { type: "turn-started" });
  const running = controller.chat("chat-a").getSnapshot();
  controller.apply("chat-a", { type: "turn-started" });
  controller.apply("unknown-chat", { type: "turn-failed" });

  expect(controller.chat("chat-a").getSnapshot()).toBe(running);
  expect(chatAListener).toHaveBeenCalledOnce();
  expect(chatBListener).not.toHaveBeenCalled();

  unsubscribeA();
  controller.apply("chat-a", { type: "turn-completed" });
  expect(chatAListener).toHaveBeenCalledOnce();
});

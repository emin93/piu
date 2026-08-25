import { expect, test, vi } from "vitest";

import type { ConversationAdapter, ConversationEvent } from "@/platform/conversations";

import { ConversationController } from "./conversation-controller";

test("the controller connects once and always sends active messages as steering", async () => {
  let emit: ((event: ConversationEvent) => void) | undefined;
  const disconnect = vi.fn();
  const prompt = vi.fn().mockResolvedValue(undefined);
  const connect = vi.fn<ConversationAdapter["connect"]>((_chatId, onEvent) => {
    emit = onEvent;
    return Promise.resolve({
      disconnect,
      snapshot: { failure: null, inputRequest: null, items: [], phase: "running" } as const,
    });
  });
  const adapter: ConversationAdapter = {
    answerInput: vi.fn().mockResolvedValue(undefined),
    connect,
    prompt,
    stop: vi.fn().mockResolvedValue(undefined),
  };
  const controller = new ConversationController("chat-42", adapter);

  await controller.connect();
  await controller.send("  Keep the tests focused.  ");
  emit?.({
    beforeItemId: null,
    type: "item-added",
    item: {
      id: "assistant-1",
      kind: "message",
      queued: false,
      role: "assistant",
      text: "On it.",
    },
  });

  expect(prompt).toHaveBeenCalledWith({
    attachments: [],
    chatId: "chat-42",
    text: "Keep the tests focused.",
    streamingBehavior: "steer",
  });
  expect(controller.store.getSnapshot().itemIds).toEqual(["assistant-1"]);

  controller.dispose();
  expect(disconnect).toHaveBeenCalledOnce();
});

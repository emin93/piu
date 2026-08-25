import { expect, test, vi } from "vitest";

import type { ConversationAdapter, ConversationEvent } from "@/platform/conversations";

import { ConversationController } from "./conversation-controller";

test("the controller connects once and always sends active messages as steering", async () => {
  let emit: ((event: ConversationEvent, revision?: number) => void) | undefined;
  const disconnect = vi.fn();
  const prompt = vi.fn().mockResolvedValue(undefined);
  const connect = vi.fn<ConversationAdapter["connect"]>((_chatId, onEvent) => {
    emit = onEvent;
    return Promise.resolve({
      disconnect,
      snapshot: {
        failure: null,
        inputRequest: null,
        items: [],
        phase: "running",
        revision: 0,
      } as const,
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
  emit?.(
    {
      beforeItemId: null,
      type: "item-added",
      item: {
        id: "assistant-1",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "On it.",
      },
    },
    1,
  );

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

test("the controller does not replay events already folded into the opening snapshot", async () => {
  let emit: ((event: ConversationEvent, revision?: number) => void) | undefined;
  const connect = vi.fn<ConversationAdapter["connect"]>((_chatId, onEvent) => {
    emit = onEvent;
    onEvent(
      {
        beforeItemId: null,
        item: {
          id: "assistant-1",
          kind: "message",
          queued: false,
          role: "assistant",
          text: "",
        },
        type: "item-added",
      },
      1,
    );
    onEvent({ delta: "Done.", itemId: "assistant-1", type: "text-delta" }, 2);
    return Promise.resolve({
      disconnect: vi.fn(),
      snapshot: {
        failure: null,
        inputRequest: null,
        items: [
          {
            id: "assistant-1",
            kind: "message",
            queued: false,
            role: "assistant",
            text: "Done.",
          },
        ],
        phase: "running",
        revision: 2,
      },
    });
  });
  const controller = new ConversationController("chat-42", {
    answerInput: vi.fn().mockResolvedValue(undefined),
    connect,
    prompt: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined),
  });

  await controller.connect();
  emit?.({ delta: "Done.", itemId: "assistant-1", type: "text-delta" }, 2);
  emit?.({ delta: " Ready.", itemId: "assistant-1", type: "text-delta" }, 3);

  expect(controller.store.getItem("assistant-1")).toMatchObject({
    text: "Done. Ready.",
  });
});

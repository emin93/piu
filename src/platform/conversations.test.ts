import type { Event } from "@tauri-apps/api/event";
import { beforeEach, expect, test, vi } from "vitest";

import type { ChatRuntimeChangedEvent } from "../generated/ChatRuntimeChangedEvent";
import type { ConversationEvent as NativeConversationEvent } from "../generated/ConversationEvent";
import type { ConversationSnapshot as NativeConversationSnapshot } from "../generated/ConversationSnapshot";

import {
  conversationErrorMessage,
  conversationRequiresCodexSignIn,
  listenToConversationEvents,
  tauriConversationAdapter,
} from "./conversations";

const boundary = vi.hoisted(() => ({
  handler: undefined as ((event: Event<ChatRuntimeChangedEvent>) => void) | undefined,
  invoke: vi.fn(),
  listen: vi.fn(),
  order: [] as string[],
  unlisten: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: boundary.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: boundary.listen }));

beforeEach(() => {
  boundary.handler = undefined;
  boundary.invoke.mockReset();
  boundary.listen.mockReset();
  boundary.order.length = 0;
  boundary.unlisten.mockReset();
  boundary.listen.mockImplementation(
    (
      eventName: string,
      handler: (event: Event<ChatRuntimeChangedEvent>) => void,
    ): Promise<() => void> => {
      boundary.order.push(`listen:${eventName}`);
      boundary.handler = handler;
      return Promise.resolve(boundary.unlisten);
    },
  );
});

test("a chat observes its first runtime event and ignores events for other chats", async () => {
  const receive = vi.fn();
  const snapshot: NativeConversationSnapshot = {
    failure: null,
    inputRequest: null,
    items: [
      {
        kind: "message",
        id: "message-0",
        queued: false,
        role: "user",
        text: "Fix the parser.",
      },
      { kind: "reasoning", id: "reasoning-1", text: "Inspecting ownership." },
      {
        kind: "tool",
        detail: "Reading parser.ts",
        id: "tool-2",
        name: "read",
        status: "running",
      },
      {
        kind: "usage",
        cacheReadTokens: 13,
        id: "usage-3",
        inputTokens: 21,
        outputTokens: 8,
      },
    ],
    phase: "running",
    revision: 7,
  };
  boundary.invoke.mockImplementationOnce((command: string) => {
    boundary.order.push(`invoke:${command}`);
    boundary.handler?.({
      payload: { chatId: "chat-other", event: { type: "turn-stopped" }, revision: 1 },
    } as Event<ChatRuntimeChangedEvent>);
    boundary.handler?.({
      payload: {
        chatId: "chat-7",
        event: {
          beforeItemId: null,
          type: "item-added",
          item: {
            kind: "message",
            id: "message-4",
            queued: false,
            role: "assistant",
            text: "Done.",
          },
        },
        revision: 8,
      },
    } as Event<ChatRuntimeChangedEvent>);
    return Promise.resolve(snapshot);
  });

  const connection = await tauriConversationAdapter.connect("chat-7", receive);

  expect(boundary.order).toEqual(["listen:chat-runtime://changed", "invoke:open_chat_runtime"]);
  expect(boundary.invoke).toHaveBeenCalledWith("open_chat_runtime", {
    request: { chatId: "chat-7" },
  });
  expect(connection.snapshot).toEqual(snapshot);
  expect(receive).toHaveBeenCalledOnce();
  expect(receive).toHaveBeenCalledWith(
    {
      beforeItemId: null,
      type: "item-added",
      item: {
        kind: "message",
        id: "message-4",
        queued: false,
        role: "assistant",
        text: "Done.",
      },
    },
    8,
  );
});

test("runtime changes preserve every generated event field", async () => {
  const receive = vi.fn<(event: NativeConversationEvent) => void>();
  boundary.invoke.mockResolvedValue({
    failure: null,
    inputRequest: null,
    items: [],
    phase: "idle",
  });
  await tauriConversationAdapter.connect("chat-7", receive);
  const events: NativeConversationEvent[] = [
    {
      beforeItemId: null,
      type: "item-added",
      item: {
        kind: "tool",
        detail: "Running tests",
        id: "tool-1",
        name: "shell",
        status: "running",
      },
    },
    { type: "item-removed", itemId: "message-optimistic" },
    { type: "text-delta", delta: "Fixed", itemId: "message-2" },
    { type: "reasoning-delta", delta: "Checking", itemId: "reasoning-3" },
    { type: "tool-update", detail: "Exit 1", itemId: "tool-1", status: "failed" },
    {
      type: "usage-update",
      cacheReadTokens: null,
      inputTokens: 34,
      itemId: "usage-4",
      outputTokens: 12,
    },
    { type: "turn-started" },
    { type: "turn-completed" },
    { type: "turn-stopped" },
    { type: "turn-failed", message: "The model route disconnected." },
  ];

  for (const event of events) {
    boundary.handler?.({ payload: { chatId: "chat-7", event } } as Event<ChatRuntimeChangedEvent>);
  }

  expect(receive.mock.calls.map(([event]) => event)).toEqual(events);
});

test("every prompt uses the host command that owns steering behavior", async () => {
  boundary.invoke.mockResolvedValue(undefined);

  await tauriConversationAdapter.prompt({
    attachments: [],
    chatId: "chat-7",
    streamingBehavior: "steer",
    text: "Check the ownership boundary.",
  });

  expect(boundary.invoke).toHaveBeenCalledOnce();
  expect(boundary.invoke).toHaveBeenCalledWith("send_chat_message", {
    request: {
      attachments: [],
      chatId: "chat-7",
      streamingBehavior: "steer",
      text: "Check the ownership boundary.",
    },
  });
});

test("stop aborts the active turn without disposing its runtime", async () => {
  boundary.invoke.mockResolvedValue(undefined);

  await tauriConversationAdapter.stop("chat-7");

  expect(boundary.invoke).toHaveBeenCalledWith("abort_chat_turn", {
    request: { chatId: "chat-7" },
  });
});

test("typed input answers cross the native command boundary unchanged", async () => {
  boundary.invoke.mockResolvedValue(undefined);

  await tauriConversationAdapter.answerInput("chat-7", "confirm-1", {
    confirmed: true,
    kind: "confirmed",
  });

  expect(boundary.invoke).toHaveBeenCalledWith("answer_conversation_input", {
    request: {
      answer: { confirmed: true, kind: "confirmed" },
      chatId: "chat-7",
      requestId: "confirm-1",
    },
  });
});

test("the global activity listener preserves chat ownership and typed input events", async () => {
  const receive = vi.fn();
  await listenToConversationEvents(receive);
  const request = {
    id: "choice-1",
    kind: "select" as const,
    message: null,
    options: ["Keep", "Replace"],
    placeholder: null,
    prefill: null,
    title: "Choose a strategy",
  };

  boundary.handler?.({
    payload: { chatId: "chat-background", event: { request, type: "input-requested" } },
  } as Event<ChatRuntimeChangedEvent>);

  expect(receive).toHaveBeenCalledWith("chat-background", {
    request,
    type: "input-requested",
  });
});

test("disconnect removes only the listener and leaves the chat runtime active", async () => {
  boundary.invoke.mockImplementation((command: string) => {
    boundary.order.push(command);
    if (command === "open_chat_runtime") {
      return Promise.resolve({ failure: null, inputRequest: null, items: [], phase: "stopped" });
    }
    return Promise.reject(new Error("unexpected command"));
  });
  boundary.unlisten.mockImplementation(() => boundary.order.push("unlisten"));
  const connection = await tauriConversationAdapter.connect("chat-7", vi.fn());

  connection.disconnect();
  connection.disconnect();
  await Promise.resolve();

  expect(boundary.order).toEqual([
    "listen:chat-runtime://changed",
    "open_chat_runtime",
    "unlisten",
  ]);
  expect(boundary.invoke).toHaveBeenCalledOnce();
  expect(boundary.invoke).toHaveBeenCalledWith("open_chat_runtime", {
    request: { chatId: "chat-7" },
  });
});

test("a runtime start failure removes the listener and keeps the typed host error", async () => {
  const failure = {
    code: "runtimeUnavailable",
    message: "Più couldn’t start the bundled agent runtime. Try opening the chat again.",
  };
  boundary.invoke.mockRejectedValue(failure);

  await expect(tauriConversationAdapter.connect("chat-7", vi.fn())).rejects.toBe(failure);

  expect(boundary.unlisten).toHaveBeenCalledOnce();
});

test("listener cleanup cannot hide a runtime start failure", async () => {
  const failure = {
    code: "setupIncomplete",
    message: "Finish the repository setup before starting the agent.",
  };
  boundary.invoke.mockRejectedValue(failure);
  boundary.unlisten.mockImplementation(() => {
    throw new Error("listener already removed");
  });

  await expect(tauriConversationAdapter.connect("chat-7", vi.fn())).rejects.toBe(failure);
});

test("a listener setup failure does not start the chat runtime", async () => {
  const failure = new Error("native event boundary unavailable");
  boundary.listen.mockRejectedValue(failure);

  await expect(tauriConversationAdapter.connect("chat-7", vi.fn())).rejects.toBe(failure);

  expect(boundary.invoke).not.toHaveBeenCalled();
  expect(boundary.unlisten).not.toHaveBeenCalled();
});

test("command failures expose only recognized host messages", () => {
  expect(
    conversationErrorMessage(
      {
        code: "conversationFailed",
        message: "Pi couldn’t accept that message. The conversation is still available.",
      },
      "The message could not be sent.",
    ),
  ).toBe("Pi couldn’t accept that message. The conversation is still available.");
  expect(
    conversationErrorMessage(
      { code: "unexpected", message: "native implementation detail" },
      "The message could not be sent.",
    ),
  ).toBe("The message could not be sent.");
  expect(conversationErrorMessage(new Error("transport detail"), "Open the chat again.")).toBe(
    "Open the chat again.",
  );
});

test("only the typed authentication boundary offers Codex sign-in", () => {
  expect(
    conversationRequiresCodexSignIn({
      code: "authenticationRequired",
      message: "Sign in to Codex to continue this conversation.",
    }),
  ).toBe(true);
  expect(
    conversationRequiresCodexSignIn({
      code: "conversationFailed",
      message: "Authentication failed for openai-codex.",
    }),
  ).toBe(false);
  expect(conversationRequiresCodexSignIn(new Error("authenticationRequired"))).toBe(false);
});

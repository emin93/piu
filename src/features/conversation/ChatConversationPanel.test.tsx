import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type {
  ConversationAdapter,
  ConversationConnection,
  ConversationEvent,
} from "@/platform/conversations";

import { ChatConversationPanel } from "./ChatConversationPanel";

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  let reject: (reason?: unknown) => void = () => undefined;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

test("events received while connecting follow the restored transcript", async () => {
  const pendingConnection = deferred<ConversationConnection>();
  let receive: ((event: ConversationEvent) => void) | undefined;
  const adapter: ConversationAdapter = {
    connect: vi.fn<ConversationAdapter["connect"]>((_chatId, onEvent) => {
      receive = onEvent;
      return pendingConnection.promise;
    }),
    prompt: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined),
  };

  render(
    <ChatConversationPanel adapter={adapter} chatId="chat-42" onRequestCodexSignIn={vi.fn()} />,
  );

  expect(screen.getByRole("status")).toHaveTextContent("Connecting to chat");

  receive?.({
    type: "item-added",
    item: { id: "live", kind: "message", role: "assistant", text: "Live event" },
  });
  act(() =>
    pendingConnection.resolve({
      disconnect: vi.fn(),
      snapshot: {
        failure: null,
        items: [{ id: "restored", kind: "message", role: "user", text: "Restored message" }],
        phase: "running",
      },
    }),
  );

  expect(await screen.findByRole("region", { name: "Conversation" })).toBeVisible();
  expect(screen.getByText("Restored message")).toBeVisible();
  expect(screen.getByText("Live event")).toBeVisible();
});

test("switching chats disconnects the previous conversation", async () => {
  const disconnectFirst = vi.fn();
  const disconnectSecond = vi.fn();
  const adapter: ConversationAdapter = {
    connect: vi.fn<ConversationAdapter["connect"]>((chatId) =>
      Promise.resolve({
        disconnect: chatId === "first" ? disconnectFirst : disconnectSecond,
        snapshot: {
          failure: null,
          items: [
            {
              id: `message-${chatId}`,
              kind: "message",
              role: "assistant",
              text: chatId === "first" ? "First conversation" : "Second conversation",
            },
          ],
          phase: "idle",
        },
      }),
    ),
    prompt: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined),
  };
  const rendered = render(
    <ChatConversationPanel adapter={adapter} chatId="first" onRequestCodexSignIn={vi.fn()} />,
  );
  expect(await screen.findByText("First conversation")).toBeVisible();

  rendered.rerender(
    <ChatConversationPanel adapter={adapter} chatId="second" onRequestCodexSignIn={vi.fn()} />,
  );

  expect(screen.getByRole("status")).toHaveTextContent("Connecting to chat");
  expect(await screen.findByText("Second conversation")).toBeVisible();
  expect(screen.queryByText("First conversation")).not.toBeInTheDocument();
  expect(disconnectFirst).toHaveBeenCalledOnce();

  rendered.unmount();
  expect(disconnectSecond).toHaveBeenCalledOnce();
});

test("a failed connection can be retried without replacing the chat", async () => {
  const user = userEvent.setup();
  const retryConnection = deferred<ConversationConnection>();
  const requestCodexSignIn = vi.fn();
  const adapter: ConversationAdapter = {
    connect: vi
      .fn<ConversationAdapter["connect"]>()
      .mockRejectedValueOnce({
        code: "runtimeUnavailable",
        message: "The Codex session is unavailable.",
      })
      .mockImplementationOnce(() => retryConnection.promise),
    prompt: vi.fn().mockRejectedValue(new Error("runtime is not connected")),
    stop: vi.fn().mockResolvedValue(undefined),
  };

  render(
    <ChatConversationPanel
      adapter={adapter}
      chatId="chat-to-retry"
      onRequestCodexSignIn={requestCodexSignIn}
    />,
  );

  expect(await screen.findByRole("alert")).toHaveTextContent("The Codex session is unavailable.");
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Your chat and isolated worktree are unchanged.",
  );
  const composer = screen.getByRole("textbox", { name: "Message Più" });
  await user.type(composer, "Keep this recovery draft");
  expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  expect(screen.getByText("Reconnect to send")).toBeVisible();
  expect(composer).toHaveValue("Keep this recovery draft");
  expect(screen.queryByRole("button", { name: "Sign in to Codex" })).not.toBeInTheDocument();
  expect(requestCodexSignIn).not.toHaveBeenCalled();

  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.getByRole("status")).toHaveTextContent("Connecting to chat");

  act(() =>
    retryConnection.resolve({
      disconnect: vi.fn(),
      snapshot: {
        failure: null,
        items: [
          {
            id: "preserved-chat",
            kind: "message",
            role: "assistant",
            text: "The same chat resumed.",
          },
        ],
        phase: "stopped",
      },
    }),
  );

  expect(await screen.findByText("The same chat resumed.")).toBeVisible();
  expect(screen.getByRole("textbox", { name: "Message Più" })).toHaveValue(
    "Keep this recovery draft",
  );
  expect(adapter.connect).toHaveBeenCalledTimes(2);
  expect(adapter.connect).toHaveBeenNthCalledWith(1, "chat-to-retry", expect.any(Function));
  expect(adapter.connect).toHaveBeenNthCalledWith(2, "chat-to-retry", expect.any(Function));
});

test("the connected panel sends steering messages and stops the active turn", async () => {
  const user = userEvent.setup();
  const prompt = vi.fn().mockResolvedValue(undefined);
  const stop = vi.fn().mockResolvedValue(undefined);
  const adapter: ConversationAdapter = {
    connect: vi.fn().mockResolvedValue({
      disconnect: vi.fn(),
      snapshot: { failure: null, items: [], phase: "running" },
    }),
    prompt,
    stop,
  };

  render(
    <ChatConversationPanel adapter={adapter} chatId="active-chat" onRequestCodexSignIn={vi.fn()} />,
  );

  const composer = await screen.findByRole("textbox", { name: "Message Più" });
  await user.type(composer, "  Keep working.  ");
  await user.click(screen.getByRole("button", { name: "Steer active turn" }));

  expect(prompt).toHaveBeenCalledWith({
    chatId: "active-chat",
    streamingBehavior: "steer",
    text: "Keep working.",
  });

  await user.click(screen.getByRole("button", { name: "Stop turn" }));
  expect(stop).toHaveBeenCalledWith("active-chat");
});

test("an authentication-related send failure keeps the draft and offers Codex sign-in", async () => {
  const user = userEvent.setup();
  const requestCodexSignIn = vi.fn();
  const adapter: ConversationAdapter = {
    connect: vi.fn().mockResolvedValue({
      disconnect: vi.fn(),
      snapshot: { failure: null, items: [], phase: "stopped" },
    }),
    prompt: vi.fn().mockRejectedValue({
      code: "authenticationRequired",
      message: "Sign in to Codex to continue this conversation.",
    }),
    stop: vi.fn().mockResolvedValue(undefined),
  };

  const rendered = render(
    <ChatConversationPanel
      adapter={adapter}
      chatId="signed-out-chat"
      onRequestCodexSignIn={requestCodexSignIn}
      revision={0}
    />,
  );

  const composer = await screen.findByRole("textbox", { name: "Message Più" });
  await user.type(composer, "Keep this message while I sign in.");
  await user.click(screen.getByRole("button", { name: "Send message" }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Sign in to Codex to continue this conversation.",
  );
  expect(composer).toHaveValue("Keep this message while I sign in.");
  await user.click(screen.getByRole("button", { name: "Sign in to Codex" }));
  expect(requestCodexSignIn).toHaveBeenCalledOnce();

  rendered.rerender(
    <ChatConversationPanel
      adapter={adapter}
      chatId="signed-out-chat"
      onRequestCodexSignIn={requestCodexSignIn}
      revision={1}
    />,
  );

  expect(await screen.findByRole("textbox", { name: "Message Più" })).toHaveValue(
    "Keep this message while I sign in.",
  );
  expect(adapter.connect).toHaveBeenCalledTimes(2);
});

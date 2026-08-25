import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";
import type {
  ConversationAdapter,
  ConversationConnection,
  ConversationEvent,
} from "@/platform/conversations";
import type { ModelControlsAdapter } from "@/platform/model-controls";

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

const qwenRoute = { modelId: "qwen3.8-27b", provider: "piu-local" };
const codexRoute = { modelId: "gpt-5.6-sol", provider: "openai-codex" };
const modelControls: ModelControlsSnapshot = {
  appliesAfterCurrentStep: false,
  efforts: ["low", "medium", "xhigh"],
  routes: [
    { acceptsImages: true, id: qwenRoute, name: "Qwen 3.8 27B" },
    { acceptsImages: true, id: codexRoute, name: "GPT-5.6 Sol" },
  ],
  selectedEffort: "medium",
  selectedRoute: qwenRoute,
};

function modelAdapter(overrides: Partial<ModelControlsAdapter> = {}): ModelControlsAdapter {
  return {
    get: vi.fn().mockResolvedValue(modelControls),
    selectEffort: vi.fn().mockResolvedValue(modelControls),
    selectRoute: vi.fn().mockResolvedValue(modelControls),
    ...overrides,
  };
}

const availableModelControls = modelAdapter();

test("events received while connecting follow the restored transcript", async () => {
  const pendingConnection = deferred<ConversationConnection>();
  let receive: ((event: ConversationEvent) => void) | undefined;
  const adapter: ConversationAdapter = {
    answerInput: vi.fn().mockResolvedValue(undefined),
    connect: vi.fn<ConversationAdapter["connect"]>((_chatId, onEvent) => {
      receive = onEvent;
      return pendingConnection.promise;
    }),
    prompt: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined),
  };

  render(
    <ChatConversationPanel
      adapter={adapter}
      chatId="chat-42"
      modelControlsAdapter={availableModelControls}
      onRequestCodexSignIn={vi.fn()}
    />,
  );

  expect(screen.getByRole("status")).toHaveTextContent("Connecting to chat");

  receive?.({
    beforeItemId: null,
    type: "item-added",
    item: {
      id: "live",
      kind: "message",
      queued: false,
      role: "assistant",
      text: "Live event",
    },
  });
  act(() =>
    pendingConnection.resolve({
      disconnect: vi.fn(),
      snapshot: {
        failure: null,
        inputRequest: null,
        items: [
          {
            id: "restored",
            kind: "message",
            queued: false,
            role: "user",
            text: "Restored message",
          },
        ],
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
    answerInput: vi.fn().mockResolvedValue(undefined),
    connect: vi.fn<ConversationAdapter["connect"]>((chatId) =>
      Promise.resolve({
        disconnect: chatId === "first" ? disconnectFirst : disconnectSecond,
        snapshot: {
          failure: null,
          inputRequest: null,
          items: [
            {
              id: `message-${chatId}`,
              kind: "message",
              queued: false,
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
    <ChatConversationPanel
      adapter={adapter}
      chatId="first"
      modelControlsAdapter={availableModelControls}
      onRequestCodexSignIn={vi.fn()}
    />,
  );
  expect(await screen.findByText("First conversation")).toBeVisible();

  rendered.rerender(
    <ChatConversationPanel
      adapter={adapter}
      chatId="second"
      modelControlsAdapter={availableModelControls}
      onRequestCodexSignIn={vi.fn()}
    />,
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
    answerInput: vi.fn().mockResolvedValue(undefined),
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
      modelControlsAdapter={availableModelControls}
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
        inputRequest: null,
        items: [
          {
            id: "preserved-chat",
            kind: "message",
            queued: false,
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
    answerInput: vi.fn().mockResolvedValue(undefined),
    connect: vi.fn().mockResolvedValue({
      disconnect: vi.fn(),
      snapshot: { failure: null, inputRequest: null, items: [], phase: "running" },
    }),
    prompt,
    stop,
  };

  render(
    <ChatConversationPanel
      adapter={adapter}
      chatId="active-chat"
      modelControlsAdapter={availableModelControls}
      onRequestCodexSignIn={vi.fn()}
    />,
  );

  const composer = await screen.findByRole("textbox", { name: "Message Più" });
  await user.type(composer, "  Keep working.  ");
  await user.click(screen.getByRole("button", { name: "Steer active turn" }));

  expect(prompt).toHaveBeenCalledWith({
    attachments: [],
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
    answerInput: vi.fn().mockResolvedValue(undefined),
    connect: vi.fn().mockResolvedValue({
      disconnect: vi.fn(),
      snapshot: { failure: null, inputRequest: null, items: [], phase: "stopped" },
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
      modelControlsAdapter={availableModelControls}
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
      modelControlsAdapter={availableModelControls}
      onRequestCodexSignIn={requestCodexSignIn}
      revision={1}
    />,
  );

  expect(await screen.findByRole("textbox", { name: "Message Più" })).toHaveValue(
    "Keep this message while I sign in.",
  );
  expect(adapter.connect).toHaveBeenCalledTimes(2);
});

test("a live Pi input request is answered through the typed conversation boundary", async () => {
  const user = userEvent.setup();
  let receive: ((event: ConversationEvent) => void) | undefined;
  const answerInput = vi.fn().mockResolvedValue(undefined);
  const adapter: ConversationAdapter = {
    answerInput,
    connect: vi.fn<ConversationAdapter["connect"]>((_chatId, onEvent) => {
      receive = onEvent;
      return Promise.resolve({
        disconnect: vi.fn(),
        snapshot: { failure: null, inputRequest: null, items: [], phase: "running" },
      });
    }),
    prompt: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined),
  };

  render(
    <ChatConversationPanel
      adapter={adapter}
      chatId="chat-input"
      modelControlsAdapter={availableModelControls}
      onRequestCodexSignIn={vi.fn()}
    />,
  );
  await screen.findByRole("region", { name: "Conversation" });
  act(() => {
    receive?.({
      request: {
        id: "confirm-1",
        kind: "confirm",
        message: "Apply the edit?",
        options: [],
        placeholder: null,
        prefill: null,
        title: "Confirm edit",
      },
      type: "input-requested",
    });
  });

  await user.click(await screen.findByRole("button", { name: "Yes" }));
  expect(answerInput).toHaveBeenCalledWith("chat-input", "confirm-1", {
    confirmed: true,
    kind: "confirmed",
  });
});

test("an active chat switches inference without disturbing its streaming transcript", async () => {
  const user = userEvent.setup();
  let receive: ((event: ConversationEvent) => void) | undefined;
  const selectRoute = vi.fn<ModelControlsAdapter["selectRoute"]>().mockResolvedValue({
    ...modelControls,
    appliesAfterCurrentStep: true,
    efforts: ["high", "max"],
    selectedEffort: "max",
    selectedRoute: codexRoute,
  });
  const models = modelAdapter({ selectRoute });
  const adapter: ConversationAdapter = {
    answerInput: vi.fn().mockResolvedValue(undefined),
    connect: vi.fn<ConversationAdapter["connect"]>((_chatId, onEvent) => {
      receive = onEvent;
      return Promise.resolve({
        disconnect: vi.fn(),
        snapshot: {
          failure: null,
          inputRequest: null,
          items: [
            {
              id: "assistant-stream",
              kind: "message",
              queued: false,
              role: "assistant",
              text: "Checking",
            },
          ],
          phase: "running",
        },
      });
    }),
    prompt: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined),
  };

  render(
    <ChatConversationPanel
      adapter={adapter}
      chatId="active-model-chat"
      modelControlsAdapter={models}
      onRequestCodexSignIn={vi.fn()}
    />,
  );

  const modelTrigger = await screen.findByRole("button", { name: "Model: Qwen 3.8 27B" });
  await user.click(modelTrigger);
  const codexItem = await screen.findByRole("menuitemradio", { name: "GPT-5.6 Sol" });
  codexItem.focus();
  act(() => {
    receive?.({ delta: " the bundle", itemId: "assistant-stream", type: "text-delta" });
  });
  expect(screen.getByText("Checking the bundle")).toBeVisible();
  expect(codexItem).toHaveFocus();

  await user.click(codexItem);
  expect(selectRoute).toHaveBeenCalledWith("active-model-chat", codexRoute);
  expect(await screen.findByRole("button", { name: "Model: GPT-5.6 Sol" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Reasoning effort: Maximum" })).toBeVisible();
  expect(screen.getByText("Switches after the current step")).toBeVisible();

  act(() => receive?.({ type: "turn-completed" }));
  expect(screen.queryByText("Switches after the current step")).not.toBeInTheDocument();
});

test("a failed route change retains the working route and offers retry", async () => {
  const user = userEvent.setup();
  const selectRoute = vi
    .fn<ModelControlsAdapter["selectRoute"]>()
    .mockRejectedValueOnce(new Error("unavailable"))
    .mockResolvedValueOnce({ ...modelControls, selectedRoute: codexRoute });
  const adapter: ConversationAdapter = {
    answerInput: vi.fn().mockResolvedValue(undefined),
    connect: vi.fn().mockResolvedValue({
      disconnect: vi.fn(),
      snapshot: { failure: null, inputRequest: null, items: [], phase: "idle" },
    }),
    prompt: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined),
  };

  render(
    <ChatConversationPanel
      adapter={adapter}
      chatId="failed-model-chat"
      modelControlsAdapter={modelAdapter({ selectRoute })}
      onRequestCodexSignIn={vi.fn()}
    />,
  );

  await user.click(await screen.findByRole("button", { name: "Model: Qwen 3.8 27B" }));
  await user.click(await screen.findByRole("menuitemradio", { name: "GPT-5.6 Sol" }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Couldn’t switch to GPT-5.6 Sol. Still using Qwen 3.8 27B.",
  );
  expect(screen.getByRole("button", { name: "Model: Qwen 3.8 27B" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Try again" }));
  expect(selectRoute).toHaveBeenCalledTimes(2);
  expect(await screen.findByRole("button", { name: "Model: GPT-5.6 Sol" })).toBeVisible();
});

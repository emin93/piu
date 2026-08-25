import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { ConversationSurface } from "./ConversationSurface";
import { ConversationStore } from "./conversation-store";

test("the transcript keeps prose plain and exposes tool detail by status", async () => {
  const user = userEvent.setup();
  const store = new ConversationStore({
    failure: null,
    phase: "running",
    items: [
      { id: "user-1", kind: "message", role: "user", text: "Check the release build." },
      { id: "reasoning-1", kind: "reasoning", text: "I should inspect the manifest." },
      { id: "assistant-1", kind: "message", role: "assistant", text: "I found two checks." },
      {
        id: "tool-success",
        kind: "tool",
        name: "Read package manifest",
        status: "succeeded",
        detail: "Read package.json",
      },
      {
        id: "tool-running",
        kind: "tool",
        name: "Build application",
        status: "running",
        detail: "Compiling the Tauri bundle",
      },
      {
        id: "tool-failed",
        kind: "tool",
        name: "Run release check",
        status: "failed",
        detail: "The release check exited with code 1",
      },
      {
        id: "usage-1",
        kind: "usage",
        inputTokens: 1200,
        outputTokens: 84,
        cacheReadTokens: 800,
      },
    ],
  });

  render(
    <ConversationSurface
      onSend={vi.fn().mockResolvedValue(undefined)}
      onStop={vi.fn().mockResolvedValue(undefined)}
      store={store}
    />,
  );

  expect(screen.getByText("Check the release build.")).toBeVisible();
  expect(screen.getByText("I found two checks.")).toBeVisible();
  expect(screen.queryByText("I should inspect the manifest.")).not.toBeInTheDocument();
  expect(screen.queryByText("Read package.json")).not.toBeInTheDocument();
  expect(screen.getByText("Compiling the Tauri bundle")).toBeVisible();
  expect(screen.getByText("The release check exited with code 1")).toBeVisible();
  expect(screen.getByText("1,200 in · 84 out · 800 cached")).toBeVisible();

  const reasoningTrigger = screen.getByRole("button", { name: "Show reasoning" });
  expect(reasoningTrigger.closest(".conversation-reasoning-row")).toHaveTextContent(
    "PiùShow reasoning",
  );

  await user.click(reasoningTrigger);
  await user.click(screen.getByRole("button", { name: "Show Read package manifest details" }));

  expect(screen.getByText("I should inspect the manifest.")).toBeVisible();
  expect(screen.getByText("Read package.json")).toBeVisible();
  expect(screen.getByRole("button", { name: "Hide Read package manifest details" })).toBeVisible();
  expect(screen.getByRole("textbox", { name: "Message Più" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Stop turn" })).toBeVisible();
});

test("streaming leaves an active steering draft untouched", async () => {
  const user = userEvent.setup();
  const onSend = vi.fn().mockResolvedValue(undefined);
  const store = new ConversationStore({
    failure: null,
    phase: "running",
    items: [
      { id: "user-1", kind: "message", role: "user", text: "Check the application." },
      { id: "assistant-1", kind: "message", role: "assistant", text: "Checking" },
    ],
  });
  render(<ConversationSurface onSend={onSend} onStop={vi.fn()} store={store} />);
  const composer = screen.getByRole("textbox", { name: "Message Più" });

  await user.type(composer, "Wait for the release build.");
  act(() => {
    store.apply({ type: "text-delta", itemId: "assistant-1", delta: " the bundle." });
  });

  expect(composer).toHaveValue("Wait for the release build.");
  expect(screen.getByText("Checking the bundle.")).toBeVisible();
  expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  expect(composer.closest("form")).toHaveAttribute("data-composer-layout", "docked");

  await user.keyboard("{Meta>}{Enter}{/Meta}");
  expect(onSend).toHaveBeenCalledWith("Wait for the release build.");
});

test("turn failure stays inline and a rejected send preserves the draft", async () => {
  const user = userEvent.setup();
  const onSend = vi.fn().mockRejectedValue(new Error("offline"));
  const store = new ConversationStore({
    failure: "The model connection closed.",
    phase: "failed",
    items: [{ id: "assistant-1", kind: "message", role: "assistant", text: "I changed" }],
  });
  render(<ConversationSurface onSend={onSend} store={store} />);
  const composer = screen.getByRole("textbox", { name: "Message Più" });

  expect(screen.getByText("Turn failed").closest("section")).toHaveTextContent(
    "The model connection closed.",
  );
  await user.type(composer, "Continue from the existing changes.");
  await user.click(screen.getByRole("button", { name: "Send message" }));

  expect(composer).toHaveValue("Continue from the existing changes.");
  expect(screen.getAllByRole("alert").at(-1)).toHaveTextContent(
    "Più couldn’t send that message. Your draft is still here.",
  );
});

test("a running tool collapses when it succeeds", () => {
  const store = new ConversationStore({
    failure: null,
    phase: "running",
    items: [
      {
        id: "tool-1",
        kind: "tool",
        name: "Run checks",
        status: "running",
        detail: "Running TypeScript",
      },
    ],
  });
  render(<ConversationSurface store={store} />);
  expect(screen.getByText("Running TypeScript")).toBeVisible();

  act(() => {
    store.apply({
      type: "tool-update",
      itemId: "tool-1",
      status: "succeeded",
      detail: "All checks passed",
    });
  });

  expect(screen.queryByText("All checks passed")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Show Run checks details" })).toBeVisible();
});

test("a stopped chat remains resumable from the same composer", () => {
  const store = new ConversationStore({
    failure: null,
    phase: "stopped",
    items: [{ id: "assistant-1", kind: "message", role: "assistant", text: "Work saved." }],
  });
  render(<ConversationSurface onSend={vi.fn()} store={store} />);

  expect(screen.getByText("Turn stopped").closest("section")).toHaveTextContent(
    "Turn stoppedSend another message to continue this chat.",
  );
  expect(screen.getByRole("textbox", { name: "Message Più" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
});

test("turn status is announced at boundaries without repeating streaming tokens", () => {
  const store = new ConversationStore({
    failure: null,
    phase: "running",
    items: [
      { id: "user-1", kind: "message", role: "user", text: "Check the application." },
      { id: "assistant-1", kind: "message", role: "assistant", text: "Checking" },
    ],
  });
  render(<ConversationSurface store={store} />);
  const turnStatus = screen.getByRole("status");

  expect(turnStatus).toHaveTextContent("Più started responding.");
  act(() => {
    store.apply({ type: "text-delta", itemId: "assistant-1", delta: " the application." });
  });
  expect(turnStatus).toHaveTextContent("Più started responding.");

  act(() => {
    store.apply({ type: "turn-completed" });
  });
  expect(turnStatus).toHaveTextContent("Più finished responding. Checking the application.");

  act(() => {
    store.apply({ type: "turn-started" });
    store.apply({ type: "turn-stopped" });
  });
  expect(turnStatus).toHaveTextContent("Più stopped responding.");

  act(() => {
    store.apply({ type: "turn-started" });
    store.apply({ type: "turn-failed", message: "The model route disconnected." });
  });
  expect(turnStatus).toHaveTextContent("Più failed to respond. The model route disconnected.");
});

test("completion does not announce an assistant message from an older turn", () => {
  const store = new ConversationStore({
    failure: null,
    phase: "running",
    items: [
      { id: "user-1", kind: "message", role: "user", text: "Finish the build." },
      { id: "assistant-1", kind: "message", role: "assistant", text: "The build is done." },
      { id: "user-2", kind: "message", role: "user", text: "Now publish it." },
    ],
  });
  render(<ConversationSurface store={store} />);

  act(() => {
    store.apply({ type: "turn-completed" });
  });

  expect(screen.getByRole("status")).toHaveTextContent("Più finished responding.");
  expect(screen.getByRole("status")).not.toHaveTextContent("The build is done.");
});

test("the transcript follows streaming content while the user is at the end", () => {
  const scrollIntoView = vi.fn();
  const disconnect = vi.fn();
  let resizeViewport: () => void = () => undefined;
  class ResizeObserverMock {
    constructor(callback: ResizeObserverCallback) {
      resizeViewport = () => callback([], this);
    }

    disconnect = disconnect;
    observe = vi.fn();
    unobserve = vi.fn();
  }
  const originalScrollIntoView = Object.getOwnPropertyDescriptor(
    Element.prototype,
    "scrollIntoView",
  );
  Object.defineProperty(Element.prototype, "scrollIntoView", {
    configurable: true,
    value: scrollIntoView,
  });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  vi.stubGlobal("ResizeObserver", ResizeObserverMock);
  const store = new ConversationStore({
    failure: null,
    phase: "running",
    items: [{ id: "assistant-1", kind: "message", role: "assistant", text: "Building" }],
  });

  const rendered = render(<ConversationSurface store={store} />);
  expect(scrollIntoView).toHaveBeenCalledWith({ block: "end" });

  resizeViewport();
  act(() => {
    store.apply({ type: "text-delta", itemId: "assistant-1", delta: " the application." });
  });
  expect(scrollIntoView).toHaveBeenCalledTimes(3);

  rendered.unmount();
  expect(disconnect).toHaveBeenCalledOnce();
  if (originalScrollIntoView) {
    Object.defineProperty(Element.prototype, "scrollIntoView", originalScrollIntoView);
  } else {
    Reflect.deleteProperty(Element.prototype, "scrollIntoView");
  }
  vi.unstubAllGlobals();
});

import { act, fireEvent, render as renderInDom, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import { mockVirtuosoAutoscrollToBottom } from "@/test/mock-virtuoso-state";

import { ConversationSurface, type TranscriptViewState } from "./ConversationSurface";
import { ConversationStore } from "./conversation-store";

let nextAnimationFrame = 1;
const animationFrames = new Map<number, FrameRequestCallback>();

function flushAnimationFrames() {
  let remainingPasses = 100;
  while (animationFrames.size > 0 && remainingPasses > 0) {
    const pending = [...animationFrames.entries()];
    animationFrames.clear();
    for (const [, callback] of pending) callback(0);
    remainingPasses -= 1;
  }
  if (animationFrames.size > 0) throw new Error("Animation frames did not settle.");
}

function render(ui: ReactElement) {
  const rendered = renderInDom(ui);
  act(flushAnimationFrames);
  return rendered;
}

beforeEach(() => {
  nextAnimationFrame = 1;
  animationFrames.clear();
  mockVirtuosoAutoscrollToBottom.mockClear();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const frameId = nextAnimationFrame;
    nextAnimationFrame += 1;
    animationFrames.set(frameId, callback);
    return frameId;
  });
  vi.stubGlobal("cancelAnimationFrame", (frameId: number) => {
    animationFrames.delete(frameId);
  });
});

afterEach(() => {
  animationFrames.clear();
  window.getSelection()?.removeAllRanges();
  vi.unstubAllGlobals();
});

test("the AI Elements transcript presents chat messages and exposes activity detail by status", async () => {
  const user = userEvent.setup();
  const store = new ConversationStore({
    failure: null,
    inputRequest: null,
    phase: "running",
    items: [
      {
        id: "user-1",
        kind: "message",
        queued: false,
        role: "user",
        text: "Check the release build.",
      },
      { id: "reasoning-1", kind: "reasoning", text: "I should inspect the manifest." },
      {
        id: "assistant-1",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "I found two checks.",
      },
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
  expect(screen.queryByText("1,200 in · 84 out · 800 cached")).not.toBeInTheDocument();

  expect(screen.getByLabelText("You")).toHaveAttribute("data-ai-element", "message");
  expect(screen.getByLabelText("Più")).toHaveAttribute("data-ai-element", "message");
  expect(screen.getByLabelText("You")).toHaveAttribute("data-role", "user");
  expect(screen.getByLabelText("Più")).toHaveAttribute("data-role", "assistant");

  const reasoningTrigger = screen.getByRole("button", { name: "Show reasoning" });
  expect(reasoningTrigger.closest("[data-ai-element='reasoning']")).toHaveTextContent("Thought");

  await user.click(reasoningTrigger);
  await user.click(screen.getByRole("button", { name: "Show Read package manifest details" }));
  await user.click(screen.getByRole("button", { name: "Show turn context" }));

  expect(screen.getByText("I should inspect the manifest.")).toBeVisible();
  expect(screen.getByText("Read package.json")).toBeVisible();
  expect(screen.getByText("1,200 in · 84 out · 800 cached")).toBeVisible();
  expect(screen.getByRole("button", { name: "Hide Read package manifest details" })).toBeVisible();
  expect(screen.getByRole("textbox", { name: "Message Più" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Stop turn" })).toBeVisible();
});

test("streaming leaves an active steering draft untouched", async () => {
  const user = userEvent.setup();
  const onSend = vi.fn().mockResolvedValue(undefined);
  const store = new ConversationStore({
    failure: null,
    inputRequest: null,
    phase: "running",
    items: [
      {
        id: "user-1",
        kind: "message",
        queued: false,
        role: "user",
        text: "Check the application.",
      },
      {
        id: "assistant-1",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "Checking",
      },
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
  expect(onSend).toHaveBeenCalledWith("Wait for the release build.", []);
});

test("an attachment-only turn previews and sends through the same composer", async () => {
  const user = userEvent.setup();
  const onSend = vi.fn().mockResolvedValue(undefined);
  const onAttachmentsChange = vi.fn();
  const attachment = {
    content: "iVBORw0KGgpmaXh0dXJl",
    id: "attachment-view",
    kind: "image" as const,
    mimeType: "image/png",
    name: "view.png",
    sizeBytes: 15,
  };
  const store = new ConversationStore({
    failure: null,
    inputRequest: null,
    items: [],
    phase: "idle",
  });
  render(
    <ConversationSurface
      attachments={[attachment]}
      draft=""
      onAttachmentsChange={onAttachmentsChange}
      onDraftChange={vi.fn()}
      onSend={onSend}
      store={store}
    />,
  );

  expect(screen.getByRole("list", { name: "Attached files" })).toHaveTextContent("view.png");
  await user.click(screen.getByRole("button", { name: "Send message" }));

  expect(onSend).toHaveBeenCalledWith("", [attachment]);
  expect(onAttachmentsChange).toHaveBeenLastCalledWith([]);
});

test("locks the accepted message while its send is pending", async () => {
  let finishSend: (() => void) | undefined;
  const pendingSend = new Promise<void>((resolve) => {
    finishSend = resolve;
  });
  const user = userEvent.setup();
  const store = new ConversationStore({
    failure: null,
    inputRequest: null,
    items: [],
    phase: "idle",
  });
  render(<ConversationSurface onSend={vi.fn().mockReturnValue(pendingSend)} store={store} />);
  const composer = screen.getByRole("textbox", { name: "Message Più" });
  await user.type(composer, "Send this exact message");
  await user.click(screen.getByRole("button", { name: "Send message" }));

  expect(composer).toHaveAttribute("readonly");
  expect(screen.getByRole("button", { name: "Attach files" })).toBeDisabled();
  await user.type(composer, " discarded edit");
  expect(composer).toHaveValue("Send this exact message");

  await act(() => {
    finishSend?.();
    return pendingSend;
  });
  expect(composer).toHaveValue("");
});

test("turn failure stays inline and a rejected send preserves the draft", async () => {
  const user = userEvent.setup();
  const onSend = vi.fn().mockRejectedValue(new Error("offline"));
  const store = new ConversationStore({
    failure: "The model connection closed.",
    inputRequest: null,
    phase: "failed",
    items: [
      {
        id: "assistant-1",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "I changed",
      },
    ],
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
    inputRequest: null,
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
    inputRequest: null,
    phase: "stopped",
    items: [
      {
        id: "assistant-1",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "Work saved.",
      },
    ],
  });
  render(<ConversationSurface onSend={vi.fn()} store={store} />);

  expect(screen.getByText("Turn stopped").closest("section")).toHaveTextContent(
    "Turn stoppedSend another message to continue this chat.",
  );
  expect(screen.getByRole("textbox", { name: "Message Più" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
});

test("queued steering and interrupted work remain explicit inline", () => {
  const store = new ConversationStore({
    failure: "The Pi runtime stopped before the turn finished.",
    inputRequest: null,
    phase: "interrupted",
    items: [
      {
        id: "user-steer",
        kind: "message",
        queued: true,
        role: "user",
        text: "Check the tests too.",
      },
      {
        detail: "Running npm test",
        id: "tool-1",
        kind: "tool",
        name: "shell",
        status: "interrupted",
      },
    ],
  });

  render(<ConversationSurface onSend={vi.fn()} store={store} />);

  expect(screen.getByText("Queued · next safe point")).toBeVisible();
  expect(screen.getByText("Interrupted", { selector: ".conversation-tool-state" })).toBeVisible();
  expect(screen.getByText("Running npm test")).toBeVisible();
  expect(screen.getByText("Turn interrupted").closest("section")).toHaveTextContent(
    "The Pi runtime stopped before the turn finished.",
  );
  expect(screen.getByRole("textbox", { name: "Message Più" })).toBeEnabled();
});

test("turn status is announced at boundaries without repeating streaming tokens", () => {
  const store = new ConversationStore({
    failure: null,
    inputRequest: null,
    phase: "running",
    items: [
      {
        id: "user-1",
        kind: "message",
        queued: false,
        role: "user",
        text: "Check the application.",
      },
      {
        id: "assistant-1",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "Checking",
      },
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
    inputRequest: null,
    phase: "running",
    items: [
      {
        id: "user-1",
        kind: "message",
        queued: false,
        role: "user",
        text: "Finish the build.",
      },
      {
        id: "assistant-1",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "The build is done.",
      },
      {
        id: "user-2",
        kind: "message",
        queued: false,
        role: "user",
        text: "Now publish it.",
      },
    ],
  });
  render(<ConversationSurface store={store} />);

  act(() => {
    store.apply({ type: "turn-completed" });
  });

  expect(screen.getByRole("status")).toHaveTextContent("Più finished responding.");
  expect(screen.getByRole("status")).not.toHaveTextContent("The build is done.");
});

test("a long transcript renders the visible tail without mounting its full history", async () => {
  const user = userEvent.setup();
  const store = new ConversationStore({
    failure: null,
    inputRequest: null,
    phase: "idle",
    items: Array.from({ length: 240 }, (_, index) => ({
      id: `message-${index + 1}`,
      kind: "message" as const,
      queued: false,
      role: index % 2 === 0 ? ("user" as const) : ("assistant" as const),
      text: `Transcript message ${index + 1}`,
    })),
  });

  render(<ConversationSurface store={store} />);

  expect(screen.getByText("Transcript message 240")).toBeVisible();
  expect(screen.queryByText("Transcript message 1")).not.toBeInTheDocument();
  expect(screen.getAllByText(/^Transcript message /).length).toBeLessThan(240);

  const composer = screen.getByRole("textbox", { name: "Message Più" });
  await user.type(composer, "The composer stays responsive.");
  expect(composer).toHaveValue("The composer stays responsive.");

  mockVirtuosoAutoscrollToBottom.mockClear();
  act(() => {
    store.apply({
      beforeItemId: null,
      item: {
        id: "message-241",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "Transcript message 241",
      },
      type: "item-added",
    });
  });
  act(flushAnimationFrames);

  expect(screen.getByText("Transcript message 241")).toBeVisible();
  expect(mockVirtuosoAutoscrollToBottom).toHaveBeenCalled();
});

function interactionTranscriptStore() {
  return new ConversationStore({
    failure: null,
    inputRequest: null,
    phase: "running",
    items: [
      ...Array.from({ length: 117 }, (_, index) => ({
        id: `history-${index}`,
        kind: "message" as const,
        queued: false,
        role: index % 2 === 0 ? ("user" as const) : ("assistant" as const),
        text: `Older transcript row ${index}`,
      })),
      { id: "reasoning-1", kind: "reasoning", text: "Inspecting the selected section." },
      {
        id: "selected-message",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "Keep this passage selected.",
      },
      {
        id: "streaming-message",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "Streaming",
      },
    ],
  });
}

function streamAndAppend(store: ConversationStore) {
  act(() => {
    store.apply({ type: "text-delta", itemId: "streaming-message", delta: " safely." });
    store.apply({
      beforeItemId: null,
      item: {
        id: "new-tail-message",
        kind: "message",
        queued: false,
        role: "assistant",
        text: "New content at the tail.",
      },
      type: "item-added",
    });
  });
  act(flushAnimationFrames);
}

test("appended turns preserve a manual scroll position", () => {
  const store = interactionTranscriptStore();

  render(<ConversationSurface store={store} />);
  const scroller = screen.getByRole("region", { name: "Conversation transcript" });
  Object.defineProperties(scroller, {
    clientHeight: { configurable: true, value: 420 },
    scrollHeight: { configurable: true, value: 10_080 },
  });
  scroller.scrollTop = 0;
  fireEvent.wheel(scroller);
  fireEvent.scroll(scroller);
  act(flushAnimationFrames);
  mockVirtuosoAutoscrollToBottom.mockClear();

  streamAndAppend(store);

  expect(screen.getByText("Streaming safely.")).toBeVisible();
  expect(screen.queryByText("New content at the tail.")).not.toBeInTheDocument();
  expect(scroller.scrollTop).toBe(0);
  expect(mockVirtuosoAutoscrollToBottom).not.toHaveBeenCalled();
});

test("a transient restore scroll signal does not replace follow intent", () => {
  const saveTranscriptState = vi.fn<(state: TranscriptViewState) => void>();
  const rendered = render(
    <ConversationSurface
      onTranscriptStateChange={saveTranscriptState}
      store={interactionTranscriptStore()}
    />,
  );
  const transcript = screen.getByRole("region", { name: "Conversation transcript" });
  Object.defineProperties(transcript, {
    clientHeight: { configurable: true, value: 420 },
    scrollHeight: { configurable: true, value: 10_080 },
  });
  transcript.scrollTop = 0;
  fireEvent.scroll(transcript);

  rendered.unmount();

  expect(saveTranscriptState.mock.calls[0]?.[0].followOutput).toBe(true);
});

test("saves and restores the virtualized transcript state across navigation", () => {
  const saveTranscriptState = vi.fn<(state: TranscriptViewState) => void>();
  const initialStore = interactionTranscriptStore();
  const rendered = render(
    <ConversationSurface onTranscriptStateChange={saveTranscriptState} store={initialStore} />,
  );
  const transcript = screen.getByRole("region", { name: "Conversation transcript" });
  Object.defineProperties(transcript, {
    clientHeight: { configurable: true, value: 420 },
    getBoundingClientRect: {
      configurable: true,
      value: () => ({ bottom: 460, height: 420, left: 0, right: 800, top: 40, width: 800 }),
    },
    scrollHeight: { configurable: true, value: 10_080 },
  });
  const anchorItem = screen.getByText("Older transcript row 112").closest("[data-item-index]");
  expect(anchorItem).not.toBeNull();
  Object.defineProperty(anchorItem, "getBoundingClientRect", {
    configurable: true,
    value: () => ({ bottom: 152, height: 84, left: 0, right: 640, top: 68, width: 640 }),
  });
  transcript.scrollTop = 144;
  fireEvent.wheel(transcript);
  fireEvent.scroll(transcript);
  act(flushAnimationFrames);

  rendered.unmount();

  const savedState = saveTranscriptState.mock.calls[0]?.[0];
  expect(savedState?.followOutput).toBe(false);
  expect(savedState?.anchor?.itemId).toBe("history-112");
  expect(savedState?.anchor?.offset).toBe(0);
  expect(savedState?.layoutSignature).toContain("selected-message:message:assistant:0:");
  expect(savedState?.virtualization).toEqual({ ranges: [], scrollTop: 144 });

  mockVirtuosoAutoscrollToBottom.mockClear();
  saveTranscriptState.mockClear();
  const restored = render(
    <ConversationSurface
      initialTranscriptState={savedState}
      onTranscriptStateChange={saveTranscriptState}
      store={interactionTranscriptStore()}
    />,
  );
  const restoredTranscript = screen.getByRole("region", { name: "Conversation transcript" });
  expect(restoredTranscript).toHaveAttribute("data-start-index", "112");
  expect(restoredTranscript).toHaveAttribute("data-start-offset", "0");
  expect(restoredTranscript).not.toHaveAttribute("data-restored-scroll-top");
  expect(mockVirtuosoAutoscrollToBottom).not.toHaveBeenCalled();

  Object.defineProperties(restoredTranscript, {
    clientHeight: { configurable: true, value: 420 },
    getBoundingClientRect: {
      configurable: true,
      value: () => ({ bottom: 500, height: 420, left: 0, right: 800, top: 80, width: 800 }),
    },
    scrollHeight: { configurable: true, value: 10_080 },
  });
  const restoredAnchorItem = screen
    .getByText("Older transcript row 112")
    .closest("[data-item-index]");
  expect(restoredAnchorItem).not.toBeNull();
  Object.defineProperty(restoredAnchorItem, "getBoundingClientRect", {
    configurable: true,
    value: () => ({ bottom: 192, height: 84, left: 0, right: 640, top: 108, width: 640 }),
  });
  restoredTranscript.scrollTop = 144;
  fireEvent.wheel(restoredTranscript);
  fireEvent.scroll(restoredTranscript);
  act(flushAnimationFrames);
  restored.unmount();

  const repeatedlySavedState = saveTranscriptState.mock.calls[0]?.[0];
  expect(repeatedlySavedState?.anchor).toEqual({ itemId: "history-112", offset: 0 });
  render(
    <ConversationSurface
      initialTranscriptState={repeatedlySavedState}
      store={interactionTranscriptStore()}
    />,
  );
  expect(screen.getByRole("region", { name: "Conversation transcript" })).toHaveAttribute(
    "data-start-offset",
    "0",
  );
});

test("does not restore stale virtualized measurements after a hidden chat streams", () => {
  const initialStore = interactionTranscriptStore();
  const saveTranscriptState = vi.fn<(state: TranscriptViewState) => void>();
  const rendered = render(
    <ConversationSurface onTranscriptStateChange={saveTranscriptState} store={initialStore} />,
  );
  const transcript = screen.getByRole("region", { name: "Conversation transcript" });
  Object.defineProperties(transcript, {
    clientHeight: { configurable: true, value: 420 },
    scrollHeight: { configurable: true, value: 10_080 },
  });
  transcript.scrollTop = 144;
  fireEvent.wheel(transcript);
  fireEvent.scroll(transcript);
  rendered.unmount();

  const backgroundStore = interactionTranscriptStore();
  backgroundStore.apply({ type: "text-delta", itemId: "streaming-message", delta: " safely." });
  render(
    <ConversationSurface
      initialTranscriptState={saveTranscriptState.mock.calls[0]?.[0]}
      store={backgroundStore}
    />,
  );

  const restoredTranscript = screen.getByRole("region", { name: "Conversation transcript" });
  expect(restoredTranscript).toHaveAttribute("data-start-index", "112");
  expect(restoredTranscript).not.toHaveAttribute("data-restored-scroll-top");
});

test("does not restore measurements after an equal-length hidden tool update", () => {
  const withRunningTool = () => {
    const store = interactionTranscriptStore();
    store.apply({
      beforeItemId: null,
      item: {
        detail: "Read alpha",
        id: "layout-tool",
        kind: "tool",
        name: "Inspect files",
        status: "running",
      },
      type: "item-added",
    });
    return store;
  };
  const initialStore = withRunningTool();
  const saveTranscriptState = vi.fn<(state: TranscriptViewState) => void>();
  const rendered = render(
    <ConversationSurface onTranscriptStateChange={saveTranscriptState} store={initialStore} />,
  );
  rendered.unmount();

  const backgroundStore = withRunningTool();
  backgroundStore.apply({
    detail: "Read bravo",
    itemId: "layout-tool",
    status: "running",
    type: "tool-update",
  });
  render(
    <ConversationSurface
      initialTranscriptState={saveTranscriptState.mock.calls[0]?.[0]}
      store={backgroundStore}
    />,
  );

  expect(screen.getByRole("region", { name: "Conversation transcript" })).not.toHaveAttribute(
    "data-restored-scroll-top",
  );
});

test("streaming and appended turns preserve an active transcript selection", () => {
  const store = interactionTranscriptStore();

  render(<ConversationSurface store={store} />);

  const selectedPassage = screen.getByText("Keep this passage selected.");
  const selection = window.getSelection();
  const range = document.createRange();
  range.selectNodeContents(selectedPassage);
  selection?.removeAllRanges();
  selection?.addRange(range);
  mockVirtuosoAutoscrollToBottom.mockClear();

  streamAndAppend(store);

  expect(screen.getByText("Streaming safely.")).toBeVisible();
  expect(screen.queryByText("New content at the tail.")).not.toBeInTheDocument();
  expect(mockVirtuosoAutoscrollToBottom).not.toHaveBeenCalled();
  expect(screen.getByText("Keep this passage selected.")).toBe(selectedPassage);
  expect(selectedPassage.contains(range.startContainer)).toBe(true);
  expect(range.startContainer.isConnected).toBe(true);
});

test("streaming and appended turns preserve keyboard focus in the transcript", () => {
  const store = interactionTranscriptStore();

  render(<ConversationSurface store={store} />);
  const focusTarget = screen.getByRole("button", { name: "Show reasoning" });
  focusTarget.focus();
  mockVirtuosoAutoscrollToBottom.mockClear();

  streamAndAppend(store);

  expect(screen.getByText("Streaming safely.")).toBeVisible();
  expect(screen.queryByText("New content at the tail.")).not.toBeInTheDocument();
  expect(mockVirtuosoAutoscrollToBottom).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(focusTarget);
});

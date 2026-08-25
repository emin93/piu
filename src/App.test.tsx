import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { App } from "./App";
import type { ConversationEvent } from "./platform/conversations";
import { installMatchMedia } from "./test/match-media";

const boundary = vi.hoisted(() => ({ verify: vi.fn() }));
const repositoryPicker = vi.hoisted(() => ({ open: vi.fn() }));
const runtimeLifecycle = vi.hoisted(() => ({
  exit: vi.fn(),
  hasActive: vi.fn(),
  shutdown: vi.fn(),
}));
const windowLifecycle = vi.hoisted(() => ({
  listen: vi.fn(),
  resolveRequest: undefined as (() => Promise<void>) | undefined,
}));
const projectInbox = vi.hoisted(() => ({
  listen: vi.fn(),
  load: vi.fn(),
  open: vi.fn(),
  remove: vi.fn(),
  saveDraft: vi.fn(),
}));
const chatWorkspaces = vi.hoisted(() => ({
  cancel: vi.fn(),
  create: vi.fn(),
  listen: vi.fn(),
  onSetup: undefined as ((event: unknown) => void) | undefined,
  openTerminal: vi.fn(),
  retry: vi.fn(),
}));
const modelAssets = vi.hoisted(() => ({
  status: vi.fn(),
  subscribe: vi.fn(),
}));
const conversationRuntime = vi.hoisted(() => ({
  answerInput: vi.fn(),
  connect: vi.fn(),
  listen: vi.fn(),
  onEvent: undefined as ((chatId: string, event: ConversationEvent) => void) | undefined,
  prompt: vi.fn(),
  stop: vi.fn(),
}));
const promptAttachments = vi.hoisted(() => ({
  select: vi.fn(),
}));

vi.mock("./platform/host-boundary", () => ({ verifyHostBoundary: boundary.verify }));
vi.mock("./platform/repository-picker", () => ({
  selectRepositoryDirectory: repositoryPicker.open,
}));
vi.mock("./platform/runtime-lifecycle", () => ({
  exitApplication: runtimeLifecycle.exit,
  hasActiveAgentTurn: runtimeLifecycle.hasActive,
  shutdownRuntimeProcesses: runtimeLifecycle.shutdown,
}));
vi.mock("./platform/window-lifecycle", () => ({
  listenToWindowClose: windowLifecycle.listen,
}));
vi.mock("./platform/project-inbox", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./platform/project-inbox")>()),
  listenToProjectInbox: projectInbox.listen,
  loadProjectInbox: projectInbox.load,
  openRepository: projectInbox.open,
  removeProject: projectInbox.remove,
  saveProjectDraft: projectInbox.saveDraft,
}));
vi.mock("./platform/chat-workspaces", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./platform/chat-workspaces")>()),
  cancelChatSetup: chatWorkspaces.cancel,
  createChat: chatWorkspaces.create,
  listenToChatSetup: chatWorkspaces.listen,
  openChatTerminal: chatWorkspaces.openTerminal,
  retryChatSetup: chatWorkspaces.retry,
}));
vi.mock("./platform/model-assets", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./platform/model-assets")>()),
  getModelAssetStatus: modelAssets.status,
  subscribeToModelAssetStatus: modelAssets.subscribe,
}));
vi.mock("./platform/conversations", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./platform/conversations")>()),
  listenToConversationEvents: conversationRuntime.listen,
  tauriConversationAdapter: {
    answerInput: conversationRuntime.answerInput,
    connect: conversationRuntime.connect,
    prompt: conversationRuntime.prompt,
    stop: conversationRuntime.stop,
  },
}));
vi.mock("./platform/prompt-attachments", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./platform/prompt-attachments")>()),
  selectPromptAttachments: promptAttachments.select,
}));
vi.mock("./features/auth/CodexSignInDialog", () => ({
  default: ({ onComplete, open }: { onComplete: () => void; open: boolean }) =>
    open ? (
      <button onClick={onComplete} type="button">
        Complete test sign-in
      </button>
    ) : null,
}));

const emptySnapshot = { projects: [], drafts: [], chats: [] };
const missingModel = {
  phase: "missing" as const,
  repository: "orcarouter/Qwen3.8-27B-Uncensored-MLX",
  revision: "0f88c40e9eff87740295f27654558fcb77e21ae5",
  manifestId: "fixture",
  totalBytes: 16_950_451_879,
  transferredBytes: 0,
  remainingBytes: 16_950_451_879,
  currentFreeBytes: 100_000_000_000,
  requiredFreeBytes: 18_024_193_703,
  currentAsset: null,
  currentFile: null,
  operationId: null,
  authenticationConfigured: false,
  canResume: false,
  availableActions: ["download"],
  errorCode: null,
  message: null,
};

beforeEach(() => {
  boundary.verify.mockReset();
  boundary.verify.mockResolvedValue({
    correlationId: "test-boundary",
    latencyMs: 2,
  });
  repositoryPicker.open.mockReset();
  repositoryPicker.open.mockResolvedValue(null);
  runtimeLifecycle.hasActive.mockReset();
  runtimeLifecycle.hasActive.mockResolvedValue(false);
  runtimeLifecycle.shutdown.mockReset();
  runtimeLifecycle.shutdown.mockResolvedValue(undefined);
  runtimeLifecycle.exit.mockReset();
  runtimeLifecycle.exit.mockResolvedValue(undefined);
  projectInbox.load.mockReset();
  projectInbox.load.mockResolvedValue(emptySnapshot);
  projectInbox.open.mockReset();
  projectInbox.remove.mockReset();
  projectInbox.saveDraft.mockReset();
  projectInbox.saveDraft.mockResolvedValue({
    attachments: [],
    projectId: 1,
    prompt: "",
    updatedAtMs: 1,
  });
  projectInbox.listen.mockReset();
  projectInbox.listen.mockResolvedValue(() => undefined);
  chatWorkspaces.cancel.mockReset();
  chatWorkspaces.cancel.mockResolvedValue(undefined);
  chatWorkspaces.create.mockReset();
  chatWorkspaces.listen.mockReset();
  chatWorkspaces.onSetup = undefined;
  chatWorkspaces.listen.mockImplementation((onSetup: (event: unknown) => void) => {
    chatWorkspaces.onSetup = onSetup;
    return Promise.resolve(() => undefined);
  });
  chatWorkspaces.openTerminal.mockReset();
  chatWorkspaces.openTerminal.mockResolvedValue({ chatId: "chat-1" });
  chatWorkspaces.retry.mockReset();
  modelAssets.status.mockReset();
  modelAssets.status.mockResolvedValue(missingModel);
  modelAssets.subscribe.mockReset();
  modelAssets.subscribe.mockResolvedValue(() => undefined);
  conversationRuntime.connect.mockReset();
  conversationRuntime.connect.mockResolvedValue({
    disconnect: vi.fn(),
    snapshot: { failure: null, inputRequest: null, items: [], phase: "idle" },
  });
  conversationRuntime.answerInput.mockReset();
  conversationRuntime.answerInput.mockResolvedValue(undefined);
  conversationRuntime.listen.mockReset();
  conversationRuntime.onEvent = undefined;
  conversationRuntime.listen.mockImplementation(
    (onEvent: (chatId: string, event: ConversationEvent) => void) => {
      conversationRuntime.onEvent = onEvent;
      return Promise.resolve(() => undefined);
    },
  );
  conversationRuntime.prompt.mockReset();
  conversationRuntime.prompt.mockResolvedValue(undefined);
  conversationRuntime.stop.mockReset();
  promptAttachments.select.mockReset();
  promptAttachments.select.mockResolvedValue({ outcome: "cancelled" });
  conversationRuntime.stop.mockResolvedValue(undefined);
  windowLifecycle.resolveRequest = undefined;
  windowLifecycle.listen.mockReset();
  windowLifecycle.listen.mockImplementation((resolveRequest: () => Promise<void>) => {
    windowLifecycle.resolveRequest = resolveRequest;
    return Promise.resolve(() => undefined);
  });
});

test("Codex sign-in reconnects the selected chat without replacing it", async () => {
  installMatchMedia("light");
  const chat = {
    id: "chat-auth",
    projectId: 1,
    projectName: "Atlas",
    title: "Continue with Codex",
    branchName: "agent/chat-auth",
    pullRequestNumber: null,
    createdAtMs: 10,
    mergeState: "unmerged" as const,
    setup: {
      phase: "succeeded" as const,
      failure: null,
      exitCode: 0,
      signal: null,
      attempt: 1,
      log: "",
    },
  };
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 1 }],
    drafts: [],
    chats: [chat],
  });
  conversationRuntime.connect
    .mockResolvedValueOnce({
      disconnect: vi.fn(),
      snapshot: { failure: null, items: [], phase: "stopped" },
    })
    .mockResolvedValueOnce({
      disconnect: vi.fn(),
      snapshot: {
        failure: null,
        items: [
          {
            id: "restored",
            kind: "message",
            queued: false,
            role: "assistant",
            text: "Signed in and restored.",
          },
        ],
        phase: "idle",
      },
    });
  conversationRuntime.prompt.mockRejectedValueOnce({
    code: "authenticationRequired",
    message: "Sign in to Codex to continue this conversation.",
  });
  const user = userEvent.setup();

  render(<App />);
  await user.click(await screen.findByRole("button", { name: /Continue with Codex/ }));
  expect(screen.getByText("Continue with Codex", { selector: ".titlebar-context" })).toBeVisible();
  const composer = await screen.findByRole("textbox", { name: "Message Più" });
  await user.type(composer, "Keep this while signing in.");
  await user.click(screen.getByRole("button", { name: "Send message" }));
  await user.click(await screen.findByRole("button", { name: "Sign in to Codex" }));
  await user.click(await screen.findByRole("button", { name: "Complete test sign-in" }));

  expect(await screen.findByText("Signed in and restored.")).toBeVisible();
  expect(screen.getByRole("textbox", { name: "Message Più" })).toHaveValue(
    "Keep this while signing in.",
  );
  expect(conversationRuntime.connect).toHaveBeenCalledTimes(2);
  expect(conversationRuntime.connect).toHaveBeenNthCalledWith(1, "chat-auth", expect.any(Function));
  expect(conversationRuntime.connect).toHaveBeenNthCalledWith(2, "chat-auth", expect.any(Function));
  expect(screen.queryByRole("button", { name: "Complete test sign-in" })).not.toBeInTheDocument();
});

test("selecting a search result opens that chat and clears the query", async () => {
  installMatchMedia("light");
  const chat = {
    id: "chat-search",
    projectId: 1,
    projectName: "Atlas",
    title: "Repair repository indexing",
    branchName: "agent/chat-search",
    pullRequestNumber: null,
    createdAtMs: 10,
    mergeState: "unmerged" as const,
    setup: {
      phase: "succeeded" as const,
      failure: null,
      exitCode: 0,
      signal: null,
      attempt: 1,
      log: "",
    },
  };
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 1 }],
    drafts: [],
    chats: [chat],
  });
  conversationRuntime.connect.mockResolvedValueOnce({
    disconnect: vi.fn(),
    snapshot: {
      failure: null,
      items: [
        {
          id: "search-result",
          kind: "message",
          queued: false,
          role: "assistant",
          text: "This is the selected chat.",
        },
      ],
      phase: "idle",
    },
  });
  const user = userEvent.setup();

  render(<App />);
  const search = await screen.findByRole("searchbox", { name: "Search chats" });
  await user.type(search, "indexing");
  await user.click(screen.getByRole("button", { name: /Repair repository indexing/ }));

  expect(search).toHaveValue("");
  expect(await screen.findByText("This is the selected chat.")).toBeVisible();
  expect(
    screen.getByText("Repair repository indexing", { selector: ".titlebar-context" }),
  ).toBeVisible();
});

test("background activity becomes unread without reordering chats and clears on selection", async () => {
  installMatchMedia("light");
  const setup = {
    attempt: 1,
    exitCode: 0,
    failure: null,
    log: "",
    phase: "succeeded" as const,
    signal: null,
  };
  projectInbox.load.mockResolvedValueOnce({
    chats: [
      {
        branchName: "agent/newer",
        createdAtMs: 20,
        id: "chat-newer",
        mergeState: "unmerged" as const,
        projectId: 1,
        projectName: "Atlas",
        pullRequestNumber: null,
        setup,
        title: "Newer chat",
      },
      {
        branchName: "agent/older",
        createdAtMs: 10,
        id: "chat-older",
        mergeState: "unmerged" as const,
        projectId: 1,
        projectName: "Atlas",
        pullRequestNumber: null,
        setup,
        title: "Older chat",
      },
    ],
    drafts: [],
    projects: [{ availability: "available", id: 1, name: "Atlas", unmergedChatCount: 2 }],
  });
  const user = userEvent.setup();

  render(<App />);
  const rows = await screen.findByRole("list", { name: "Active chats" });
  expect(
    Array.from(rows.querySelectorAll<HTMLElement>("[data-chat-id]")).map(
      (row) => row.dataset.chatId,
    ),
  ).toEqual(["chat-newer", "chat-older"]);

  act(() => {
    conversationRuntime.onEvent?.("chat-older", {
      request: {
        id: "input-1",
        kind: "confirm",
        message: "Continue?",
        options: [],
        placeholder: null,
        prefill: null,
        title: "Confirm change",
      },
      type: "input-requested",
    });
  });

  const olderRow = rows.querySelector<HTMLElement>('[data-chat-id="chat-older"]');
  expect(olderRow).toHaveAttribute("data-activity", "needs-input");
  expect(olderRow).toHaveAttribute("data-unread", "true");
  expect(screen.getByRole("button", { name: "Older chat, needs-input, unread" })).toBeVisible();
  expect(
    Array.from(rows.querySelectorAll<HTMLElement>("[data-chat-id]")).map(
      (row) => row.dataset.chatId,
    ),
  ).toEqual(["chat-newer", "chat-older"]);

  await user.click(screen.getByRole("button", { name: "Older chat, needs-input, unread" }));
  expect(olderRow).not.toHaveAttribute("data-unread");

  const search = screen.getByRole("searchbox", { name: "Search chats" });
  await user.type(search, "Older");
  act(() => {
    conversationRuntime.onEvent?.("chat-older", { type: "turn-completed" });
  });
  expect(rows.querySelector('[data-chat-id="chat-older"]')).toHaveAttribute("data-unread", "true");
  await user.clear(search);
  expect(rows.querySelector('[data-chat-id="chat-older"]')).not.toHaveAttribute("data-unread");

  act(() => {
    conversationRuntime.onEvent?.("chat-newer", {
      message: "The Pi runtime stopped unexpectedly.",
      type: "turn-interrupted",
    });
  });
  const newerRow = rows.querySelector<HTMLElement>('[data-chat-id="chat-newer"]');
  expect(newerRow).toHaveAttribute("data-activity", "interrupted");
  expect(newerRow).toHaveAttribute("data-unread", "true");
});

test("first send creates a durable chat and moves into streamed setup", async () => {
  installMatchMedia("light");
  const project = { id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 };
  projectInbox.load.mockResolvedValueOnce({ projects: [project], drafts: [], chats: [] });
  const chat = {
    id: "chat-1",
    projectId: 1,
    projectName: "Atlas",
    title: "Repair the parser",
    branchName: "agent/chat-1-repair-the-parser",
    pullRequestNumber: null,
    createdAtMs: 10,
    mergeState: "unmerged",
    setup: {
      phase: "running",
      failure: null,
      exitCode: null,
      signal: null,
      attempt: 1,
      log: "Installing dependencies\n",
    },
  };
  chatWorkspaces.create.mockResolvedValueOnce({
    chat,
    snapshot: { projects: [{ ...project, unmergedChatCount: 1 }], drafts: [], chats: [chat] },
  });
  const user = userEvent.setup();
  render(<App />);
  const composer = await screen.findByRole("textbox", { name: "Draft for Atlas" });
  await user.type(composer, "Repair the parser");

  await user.click(screen.getByRole("button", { name: "Send message" }));

  expect(chatWorkspaces.create).toHaveBeenCalledWith(1, "Repair the parser", []);
  expect(await screen.findByRole("heading", { name: "Setting up worktree" })).toBeVisible();
  expect(screen.getByLabelText("Setup output")).toHaveTextContent("Installing dependencies");
  expect(screen.queryByRole("textbox", { name: "Draft for Atlas" })).not.toBeInTheDocument();

  act(() => {
    chatWorkspaces.onSetup?.({
      chatId: chat.id,
      setup: { ...chat.setup, phase: "failed", failure: "exit", exitCode: 7 },
    });
  });
  expect(await screen.findByRole("heading", { name: "Setup failed" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Retry setup" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Open Terminal" })).toBeVisible();
});

test("a selected attachment is previewed, persisted, and included in chat creation", async () => {
  installMatchMedia("light");
  const project = { id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 };
  const attachment = {
    content: "public boundary",
    id: "attachment-notes",
    kind: "text" as const,
    mimeType: "text/plain",
    name: "notes.txt",
    sizeBytes: 15,
  };
  projectInbox.load.mockResolvedValueOnce({ projects: [project], drafts: [], chats: [] });
  promptAttachments.select.mockResolvedValueOnce({
    attachments: [attachment],
    outcome: "selected",
  });
  const chat = {
    id: "chat-attachment",
    projectId: 1,
    projectName: "Atlas",
    title: "Use the notes",
    branchName: "agent/chat-attachment",
    pullRequestNumber: null,
    createdAtMs: 10,
    mergeState: "unmerged",
    setup: {
      phase: "pending",
      failure: null,
      exitCode: null,
      signal: null,
      attempt: 0,
      log: "",
    },
  };
  chatWorkspaces.create.mockResolvedValueOnce({
    chat,
    snapshot: { projects: [{ ...project, unmergedChatCount: 1 }], drafts: [], chats: [chat] },
  });
  const user = userEvent.setup();
  render(<App />);

  await user.click(await screen.findByRole("button", { name: "Attach files" }));
  expect(screen.getByRole("list", { name: "Attached files" })).toHaveTextContent("notes.txt");
  await waitFor(() => expect(projectInbox.saveDraft).toHaveBeenCalledWith(1, "", [attachment]));

  await user.type(screen.getByRole("textbox", { name: "Draft for Atlas" }), "Use the notes");
  await user.click(screen.getByRole("button", { name: "Send message" }));
  expect(chatWorkspaces.create).toHaveBeenCalledWith(1, "Use the notes", [attachment]);
});

test("Settings preserves the selected project and its draft when returning to Inbox", async () => {
  installMatchMedia("light");
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
    drafts: [{ attachments: [], projectId: 1, prompt: "Keep this draft", updatedAtMs: 1 }],
    chats: [],
  });
  const user = userEvent.setup();

  render(<App />);

  const draft = await screen.findByRole("textbox", { name: "Draft for Atlas" });
  await user.click(screen.getByRole("button", { name: /Atlas, available/ }));
  await user.type(draft, " while away");
  await user.click(screen.getByRole("button", { name: "Settings" }));

  expect(await screen.findByRole("heading", { name: "Models & Resources" })).toBeVisible();
  const backToInbox = screen.getByRole("button", { name: "Back to Inbox" });
  await waitFor(() => expect(backToInbox).toHaveFocus());
  expect(screen.getByText("Models & Resources", { selector: ".titlebar-context" })).toBeVisible();
  expect(projectInbox.saveDraft).toHaveBeenCalledWith(1, "Keep this draft while away", []);

  await user.click(backToInbox);
  expect(await screen.findByRole("textbox", { name: "Draft for Atlas" })).toHaveValue(
    "Keep this draft while away",
  );
  expect(screen.getByRole("button", { name: /Atlas, available/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByText("Atlas", { selector: ".titlebar-context" })).toBeVisible();
  await waitFor(() => expect(screen.getByRole("button", { name: "Settings" })).toHaveFocus());
});

test("the shell follows system appearance changes live", () => {
  const systemAppearance = installMatchMedia("dark");

  render(<App />);
  expect(document.documentElement).toHaveAttribute("data-appearance", "dark");

  act(() => systemAppearance.setAppearance("light"));
  expect(document.documentElement).toHaveAttribute("data-appearance", "light");
});

test("the empty inbox action is keyboard reachable", async () => {
  installMatchMedia("light");
  const openRepository = vi.fn();
  const user = userEvent.setup();

  render(<App onOpenRepository={openRepository} />);

  expect(await screen.findByRole("heading", { name: "Open a repository to start" })).toBeVisible();
  const action = screen.getByRole("button", { name: "Open Repository" });
  await user.tab();
  await user.tab();
  expect(action).toHaveFocus();
  await user.keyboard("{Enter}");
  expect(openRepository).toHaveBeenCalledOnce();
});

test("launch keeps All Projects selected and focuses the first available project draft", async () => {
  installMatchMedia("light");
  projectInbox.load.mockResolvedValueOnce({
    projects: [
      { id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 },
      { id: 2, name: "Beacon", availability: "available", unmergedChatCount: 0 },
    ],
    drafts: [{ attachments: [], projectId: 1, prompt: "Continue the parser work", updatedAtMs: 1 }],
    chats: [],
  });

  render(<App />);

  expect(await screen.findByRole("button", { name: "All Projects, 2 projects" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  const composer = screen.getByRole("textbox", { name: "Draft for Atlas" });
  expect(composer).toHaveValue("Continue the parser work");
  await waitFor(() => expect(composer).toHaveFocus());
});

test("launch without repositories never exposes an unsendable prompt", async () => {
  installMatchMedia("dark");

  render(<App />);

  expect(await screen.findByRole("button", { name: "Open Repository" })).toBeVisible();
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
});

test("the production empty inbox action opens the native repository picker", async () => {
  installMatchMedia("light");
  const user = userEvent.setup();

  render(<App />);
  await user.click(await screen.findByRole("button", { name: "Open Repository" }));

  expect(repositoryPicker.open).toHaveBeenCalledOnce();
});

test("a selected repository is validated by the host and focused as a project", async () => {
  installMatchMedia("light");
  repositoryPicker.open.mockResolvedValueOnce("/private/repositories/atlas");
  projectInbox.open.mockResolvedValueOnce({
    focusedProjectId: 1,
    outcome: "added",
    snapshot: {
      projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
      drafts: [],
      chats: [],
    },
  });
  const user = userEvent.setup();

  render(<App />);
  await user.click(await screen.findByRole("button", { name: "Open Repository" }));

  expect(projectInbox.open).toHaveBeenCalledWith("/private/repositories/atlas");
  expect(await screen.findByRole("textbox", { name: "Draft for Atlas" })).toBeVisible();
  expect(screen.queryByText("/private/repositories/atlas")).not.toBeInTheDocument();
});

test("an invalid repository produces an actionable inline error and is not shown", async () => {
  installMatchMedia("light");
  repositoryPicker.open.mockResolvedValueOnce("/private/not-a-repository");
  projectInbox.open.mockRejectedValueOnce({
    code: "invalidRepository",
    message: "Choose a folder that contains a Git repository.",
  });
  const user = userEvent.setup();

  render(<App />);
  await user.click(await screen.findByRole("button", { name: "Open Repository" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("contains a Git repository");
  expect(screen.queryByText("not-a-repository")).not.toBeInTheDocument();
});

test("draft changes update immediately and cross the persistence boundary", async () => {
  installMatchMedia("light");
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
    drafts: [],
    chats: [],
  });
  const user = userEvent.setup();

  render(<App />);
  await user.click(await screen.findByRole("button", { name: /Atlas, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Atlas" }), "Fix parsing");

  expect(screen.getByRole("textbox", { name: "Draft for Atlas" })).toHaveValue("Fix parsing");
  expect(screen.getByText("Saving")).toBeVisible();
  await waitFor(() =>
    expect(projectInbox.saveDraft).toHaveBeenLastCalledWith(1, "Fix parsing", []),
  );
  expect(projectInbox.saveDraft).toHaveBeenCalledTimes(1);
  expect(await screen.findByText("Saved locally")).toBeVisible();
});

test("draft navigation flushes the pending value before the debounce", async () => {
  installMatchMedia("light");
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
    drafts: [],
    chats: [],
  });
  const user = userEvent.setup();

  render(<App />);
  await user.click(await screen.findByRole("button", { name: /Atlas, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Atlas" }), "Flush me");
  await user.click(screen.getByRole("button", { name: "All Projects, 1 project" }));

  expect(projectInbox.saveDraft).toHaveBeenCalledWith(1, "Flush me", []);
});

test("a native close waits for the pending draft flush and owned runtimes", async () => {
  installMatchMedia("light");
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
    drafts: [],
    chats: [],
  });
  const user = userEvent.setup();
  render(<App />);
  await user.click(await screen.findByRole("button", { name: /Atlas, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Atlas" }), "Close safely");

  await act(async () => {
    await windowLifecycle.resolveRequest?.();
  });

  expect(runtimeLifecycle.hasActive).toHaveBeenCalledOnce();
  expect(projectInbox.saveDraft).toHaveBeenCalledWith(1, "Close safely", []);
  expect(runtimeLifecycle.shutdown).toHaveBeenCalledOnce();
  expect(runtimeLifecycle.exit).toHaveBeenCalledOnce();
});

test("cancelling an active-turn close leaves the agent and window running", async () => {
  installMatchMedia("light");
  runtimeLifecycle.hasActive.mockResolvedValue(true);
  const user = userEvent.setup();
  render(<App />);

  await act(async () => {
    await windowLifecycle.resolveRequest?.();
  });

  expect(await screen.findByRole("alertdialog")).toHaveTextContent(
    "Active agent work will be stopped",
  );
  await user.click(screen.getByRole("button", { name: "Keep working" }));

  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  expect(runtimeLifecycle.shutdown).not.toHaveBeenCalled();
  expect(runtimeLifecycle.exit).not.toHaveBeenCalled();
});

test("confirming an active-turn close persists drafts, stops runtimes, and exits the app", async () => {
  installMatchMedia("light");
  runtimeLifecycle.hasActive.mockResolvedValue(true);
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
    drafts: [],
    chats: [],
  });
  const user = userEvent.setup();
  render(<App />);
  await user.click(await screen.findByRole("button", { name: /Atlas, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Atlas" }), "Close safely");

  await act(async () => {
    await windowLifecycle.resolveRequest?.();
  });
  await user.click(await screen.findByRole("button", { name: "Stop and quit" }));

  await waitFor(() => expect(runtimeLifecycle.exit).toHaveBeenCalledOnce());
  expect(projectInbox.saveDraft).toHaveBeenCalledWith(1, "Close safely", []);
  expect(runtimeLifecycle.shutdown).toHaveBeenCalledOnce();
});

test("a failed draft save never claims the draft is saved", async () => {
  installMatchMedia("light");
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
    drafts: [],
    chats: [],
  });
  projectInbox.saveDraft.mockRejectedValueOnce(new Error("sqlite unavailable"));
  const user = userEvent.setup();

  render(<App />);
  await user.click(await screen.findByRole("button", { name: /Atlas, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Atlas" }), "Keep me");

  expect(await screen.findByText(/Couldn't save this draft/)).toBeVisible();
  expect(screen.queryByText("Saved locally")).not.toBeInTheDocument();
  await expect(windowLifecycle.resolveRequest?.()).rejects.toThrow("could not be saved");
});

test("a repository picker failure is explained in product language", async () => {
  installMatchMedia("light");
  repositoryPicker.open.mockRejectedValueOnce(new Error("dialog unavailable"));
  const user = userEvent.setup();

  render(<App />);
  await user.click(await screen.findByRole("button", { name: "Open Repository" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't open the repository picker");
});

test("the shell verifies the typed host boundary without exposing internals", async () => {
  installMatchMedia("light");

  render(<App />);

  await waitFor(() => expect(boundary.verify).toHaveBeenCalledOnce());
  expect(screen.queryByText(/core ready|schema 1/i)).not.toBeInTheDocument();
});

test("the complete visible titlebar delegates deep dragging to Tauri", async () => {
  installMatchMedia("light");

  render(<App />);

  await screen.findByRole("main", { name: "Più inbox" });
  const titlebar = document.querySelector<HTMLElement>(".titlebar");
  expect(titlebar).toHaveAttribute("data-tauri-drag-region", "deep");
});

test("startup presents a stable non-interactive loading state", () => {
  installMatchMedia("light");

  render(<App visualReviewState="loading" />);

  expect(screen.getByRole("status")).toHaveTextContent("Opening your inbox");
  expect(screen.queryByRole("button", { name: "Open Repository" })).not.toBeInTheDocument();
  expect(boundary.verify).not.toHaveBeenCalled();
});

test("the close confirmation has a deterministic visual review state", async () => {
  installMatchMedia("light");

  render(<App visualReviewState="closeConfirmation" />);

  expect(await screen.findByRole("alertdialog")).toHaveTextContent("Stop active work and quit?");
  expect(screen.getByRole("button", { name: "Keep working" })).toBeEnabled();
});

test("the conversation visual review state opens the first chat", async () => {
  installMatchMedia("light");
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 1 }],
    drafts: [],
    chats: [
      {
        id: "chat-review",
        projectId: 1,
        projectName: "Atlas",
        title: "Review the runtime",
        branchName: "agent/chat-review",
        pullRequestNumber: null,
        createdAtMs: 10,
        mergeState: "unmerged" as const,
        setup: {
          phase: "succeeded" as const,
          failure: null,
          exitCode: 0,
          signal: null,
          attempt: 1,
          log: "",
        },
      },
    ],
  });
  conversationRuntime.connect.mockResolvedValueOnce({
    disconnect: vi.fn(),
    snapshot: {
      failure: null,
      items: [
        {
          id: "review-response",
          kind: "message",
          queued: false,
          role: "assistant",
          text: "The packaged runtime is responding.",
        },
      ],
      phase: "running",
    },
  });

  render(<App visualReviewState="conversation" />);

  expect(await screen.findByText("The packaged runtime is responding.")).toBeVisible();
  expect(conversationRuntime.connect).toHaveBeenCalledWith("chat-review", expect.any(Function));
});

test("a host startup failure offers a retry in product language", async () => {
  installMatchMedia("light");
  boundary.verify.mockRejectedValueOnce(new Error("IPC unavailable"));
  const user = userEvent.setup();

  render(<App />);

  expect(await screen.findByRole("heading", { name: "Più couldn't start" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(boundary.verify).toHaveBeenCalledTimes(2));
  await waitFor(() =>
    expect(screen.queryByRole("heading", { name: "Più couldn't start" })).not.toBeInTheDocument(),
  );
});

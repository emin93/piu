import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { App } from "./App";
import { installMatchMedia } from "./test/match-media";

const boundary = vi.hoisted(() => ({ verify: vi.fn() }));
const repositoryPicker = vi.hoisted(() => ({ open: vi.fn() }));
const windowLifecycle = vi.hoisted(() => ({
  beforeClose: undefined as (() => Promise<void>) | undefined,
  listen: vi.fn(),
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

vi.mock("./platform/host-boundary", () => ({ verifyHostBoundary: boundary.verify }));
vi.mock("./platform/repository-picker", () => ({
  selectRepositoryDirectory: repositoryPicker.open,
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
  projectInbox.load.mockReset();
  projectInbox.load.mockResolvedValue(emptySnapshot);
  projectInbox.open.mockReset();
  projectInbox.remove.mockReset();
  projectInbox.saveDraft.mockReset();
  projectInbox.saveDraft.mockResolvedValue({ projectId: 1, prompt: "", updatedAtMs: 1 });
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
  windowLifecycle.beforeClose = undefined;
  windowLifecycle.listen.mockReset();
  windowLifecycle.listen.mockImplementation((beforeClose: () => Promise<void>) => {
    windowLifecycle.beforeClose = beforeClose;
    return Promise.resolve(() => undefined);
  });
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

  expect(chatWorkspaces.create).toHaveBeenCalledWith(1, "Repair the parser");
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

test("Settings preserves the selected project and its draft when returning to Inbox", async () => {
  installMatchMedia("light");
  projectInbox.load.mockResolvedValueOnce({
    projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
    drafts: [{ projectId: 1, prompt: "Keep this draft", updatedAtMs: 1 }],
    chats: [],
  });
  const user = userEvent.setup();

  render(<App />);

  const draft = await screen.findByRole("textbox", { name: "Draft for Atlas" });
  await user.click(screen.getByRole("button", { name: /Atlas, available/ }));
  await user.type(draft, " while away");
  await user.click(screen.getByRole("button", { name: "Settings" }));

  expect(await screen.findByRole("heading", { name: "Settings" })).toBeVisible();
  const backToInbox = screen.getByRole("button", { name: "Back to Inbox" });
  await waitFor(() => expect(backToInbox).toHaveFocus());
  expect(screen.getByText("Settings", { selector: ".titlebar-context" })).toBeVisible();
  expect(projectInbox.saveDraft).toHaveBeenCalledWith(1, "Keep this draft while away");

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
    drafts: [{ projectId: 1, prompt: "Continue the parser work", updatedAtMs: 1 }],
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
  await waitFor(() => expect(projectInbox.saveDraft).toHaveBeenLastCalledWith(1, "Fix parsing"));
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

  expect(projectInbox.saveDraft).toHaveBeenCalledWith(1, "Flush me");
});

test("a native close waits for the pending draft flush", async () => {
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
    await windowLifecycle.beforeClose?.();
  });

  expect(projectInbox.saveDraft).toHaveBeenCalledWith(1, "Close safely");
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

  expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't save this draft");
  expect(screen.queryByText("Saved locally")).not.toBeInTheDocument();
  await expect(windowLifecycle.beforeClose?.()).rejects.toThrow("could not be saved");
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

test("startup presents a stable non-interactive loading state", () => {
  installMatchMedia("light");

  render(<App visualReviewStartup="loading" />);

  expect(screen.getByRole("status")).toHaveTextContent("Opening your inbox");
  expect(screen.queryByRole("button", { name: "Open Repository" })).not.toBeInTheDocument();
  expect(boundary.verify).not.toHaveBeenCalled();
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

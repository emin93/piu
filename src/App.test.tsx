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

const emptySnapshot = { projects: [], drafts: [], chats: [] };

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
  windowLifecycle.beforeClose = undefined;
  windowLifecycle.listen.mockReset();
  windowLifecycle.listen.mockImplementation((beforeClose: () => Promise<void>) => {
    windowLifecycle.beforeClose = beforeClose;
    return Promise.resolve(() => undefined);
  });
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

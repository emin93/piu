import { useEffect, useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import type { ModelRouteId } from "@/generated/ModelRouteId";
import type { ReasoningEffort } from "@/generated/ReasoningEffort";
import type { ConversationAdapter } from "@/platform/conversations";
import type { ModelControlsAdapter } from "@/platform/model-controls";
import type { PromptAttachment } from "@/platform/prompt-attachments";
import type { InboxSnapshot } from "@/platform/project-inbox";

import { ChatActivityController } from "./chat-activity-controller";
import { ProjectDraftController } from "./draft-controller";
import { readRememberedProjectScope, rememberProjectScope } from "./inbox-scope";
import { InboxWorkspace } from "./InboxWorkspace";
import { ChatSetupController } from "./setup-controller";

const readySetup = {
  attempt: 1,
  exitCode: 0,
  failure: null,
  log: "",
  phase: "succeeded" as const,
  signal: null,
};

const conversationAdapter: ConversationAdapter = {
  answerInput: vi.fn().mockResolvedValue(undefined),
  connect: vi.fn().mockResolvedValue({
    disconnect: vi.fn(),
    snapshot: { failure: null, inputRequest: null, items: [], phase: "idle" },
  }),
  prompt: vi.fn().mockResolvedValue(undefined),
  stop: vi.fn().mockResolvedValue(undefined),
};

const selectedRoute = { modelId: "qwen3.8-27b", provider: "local-mlx" };
const modelControlsAdapter: ModelControlsAdapter<number> = {
  get: vi.fn().mockResolvedValue({
    appliesAfterCurrentStep: false,
    efforts: ["low", "medium", "xhigh"],
    routes: [{ acceptsImages: false, id: selectedRoute, name: "Qwen 3.8 27B" }],
    selectedEffort: "medium",
    selectedRoute,
  }),
  selectEffort: vi.fn(),
  selectRoute: vi.fn(),
};

const populatedSnapshot: InboxSnapshot = {
  projects: [
    { availability: "available", id: 1, name: "Atlas", unmergedChatCount: 2 },
    { availability: "missing", id: 2, name: "Beacon", unmergedChatCount: 1 },
    { availability: "available", id: 3, name: "Caldera", unmergedChatCount: 0 },
  ],
  drafts: [{ attachments: [], projectId: 1, prompt: "Explain the parser", updatedAtMs: 500 }],
  chats: [
    {
      branchName: "docs/importer",
      createdAtMs: 100,
      id: "older",
      mergeState: "unmerged",
      projectId: 1,
      projectName: "Atlas",
      pullRequestNumber: null,
      setup: readySetup,
      title: "Document the importer",
    },
    {
      branchName: "fix/indexing",
      createdAtMs: 300,
      id: "newer",
      mergeState: "unmerged",
      projectId: 2,
      projectName: "Beacon",
      pullRequestNumber: 73,
      setup: readySetup,
      title: "Repair repository indexing",
    },
    {
      branchName: "feature/stable-order",
      createdAtMs: 200,
      id: "middle",
      mergeState: "unmerged",
      projectId: 1,
      projectName: "Atlas",
      pullRequestNumber: 62,
      setup: readySetup,
      title: "Keep this deliberately long chat title stable while its row truncates cleanly",
    },
    {
      branchName: "agent/history",
      createdAtMs: 50,
      id: "merged",
      mergeState: "merged",
      projectId: null,
      projectName: "Removed project",
      pullRequestNumber: 41,
      setup: readySetup,
      title: "Historical result",
    },
  ],
};

type CreateChat = (
  projectId: number,
  prompt: string,
  attachments: readonly PromptAttachment[],
  route: ModelRouteId,
  effort: ReasoningEffort,
) => Promise<string | undefined>;

function WorkspaceHarness({
  initialChatId = null,
  initialProjectId = null,
  initialSnapshot = populatedSnapshot,
  onCreate = vi.fn<CreateChat>().mockResolvedValue(undefined),
  onDelete = vi.fn<(chatId: string) => Promise<string | undefined>>().mockResolvedValue(undefined),
  onRename = vi
    .fn<(chatId: string, title: string) => Promise<string | undefined>>()
    .mockResolvedValue(undefined),
}: {
  initialChatId?: string | null;
  initialProjectId?: number | null;
  initialSnapshot?: InboxSnapshot;
  onCreate?: CreateChat;
  onDelete?: (chatId: string) => Promise<string | undefined>;
  onRename?: (chatId: string, title: string) => Promise<string | undefined>;
}) {
  const [snapshot, setSnapshot] = useState(initialSnapshot);
  const [selectedProjectId, setSelectedProjectId] = useState(initialProjectId);
  const [selectedChatId, setSelectedChatId] = useState<string | null>(initialChatId);
  const [query, setQuery] = useState("");
  const [drafts] = useState(() => {
    const controller = new ProjectDraftController(() => Promise.resolve());
    controller.reconcile(initialSnapshot);
    return controller;
  });
  const [setups] = useState(() => {
    const controller = new ChatSetupController();
    controller.reconcile(initialSnapshot);
    return controller;
  });
  const [activities] = useState(() => new ChatActivityController());
  useEffect(() => drafts.reconcile(snapshot), [drafts, snapshot]);
  useEffect(() => setups.reconcile(snapshot), [setups, snapshot]);

  const deleteChat = async (chatId: string) => {
    const error = await onDelete(chatId);
    if (error) return error;
    setSnapshot((current) => ({
      ...current,
      chats: current.chats.filter(({ id }) => id !== chatId),
    }));
    if (selectedChatId === chatId) setSelectedChatId(null);
    return undefined;
  };

  return (
    <InboxWorkspace
      actionError={undefined}
      activities={activities}
      conversationAdapter={conversationAdapter}
      conversationRevision={0}
      drafts={drafts}
      modelControlsAdapter={modelControlsAdapter}
      onCancelSetup={vi.fn().mockResolvedValue(undefined)}
      onCreateChat={onCreate}
      onDeleteChat={deleteChat}
      onNewChat={() => {
        setSelectedChatId(null);
        setQuery("");
      }}
      onOpenRepository={vi.fn()}
      onOpenSettings={vi.fn()}
      onOpenTerminal={vi.fn().mockResolvedValue(undefined)}
      onProjectScopeChange={setSelectedProjectId}
      onQueryChange={setQuery}
      onRenameChat={onRename}
      onRequestCodexSignIn={vi.fn()}
      onRetrySetup={vi.fn().mockResolvedValue(undefined)}
      onSelectChat={setSelectedChatId}
      query={query}
      selectedChatId={selectedChatId}
      selectedProjectId={selectedProjectId}
      setups={setups}
      snapshot={snapshot}
    />
  );
}

beforeEach(() => {
  window.localStorage.clear();
  vi.clearAllMocks();
});

test("renders a compact header and a headerless flat newest-created-first inbox", () => {
  render(<WorkspaceHarness />);

  expect(screen.getByRole("searchbox", { name: "Search chats" })).toBeVisible();
  expect(screen.getByRole("button", { name: "New Chat" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Project scope: All Projects" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Open Repository" })).toBeVisible();
  expect(screen.queryByRole("heading", { name: /Projects|Drafts|Chats/ })).not.toBeInTheDocument();

  const inbox = screen.getByRole("list", { name: "Chat inbox" });
  expect(
    within(inbox)
      .getAllByRole("listitem")
      .map((row) => row.dataset.chatId),
  ).toEqual(["newer", "middle", "older"]);
  expect(within(inbox).getByText("Beacon")).toBeVisible();
  expect(within(inbox).getAllByText("Atlas")).toHaveLength(2);
});

test("changes scope without closing the current chat and exposes one inline project draft", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness initialChatId="newer" />);

  await user.click(screen.getByRole("button", { name: "Project scope: All Projects" }));
  await user.click(await screen.findByRole("menuitemradio", { name: "Atlas" }));

  expect(screen.getByRole("button", { name: "Project scope: Atlas" })).toBeVisible();
  expect(screen.getByText("Explain the parser")).toBeVisible();
  expect(screen.getByText("Draft")).toBeVisible();
  expect(
    within(screen.getByRole("list", { name: "Chat inbox" })).queryByText("Beacon"),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Chat workspace" })).toHaveAttribute(
    "data-selected-chat-id",
    "newer",
  );

  await user.click(screen.getByRole("button", { name: "Project scope: Atlas" }));
  await user.click(await screen.findByRole("menuitemradio", { name: "All Projects" }));
  expect(screen.queryByText("Draft")).not.toBeInTheDocument();
});

test("stores and reads a versioned project scope without inventing a missing scope", () => {
  expect(readRememberedProjectScope()).toBeNull();
  rememberProjectScope(3);
  expect(readRememberedProjectScope()).toBe(3);
  rememberProjectScope(null);
  expect(readRememberedProjectScope()).toBeNull();
  window.localStorage.setItem("piu.inbox-scope.v1", "not-a-project");
  expect(readRememberedProjectScope()).toBeNull();
});

test("search filters metadata in stable order, stays sidebar-only, and Escape clears it", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness initialChatId="older" />);

  const search = screen.getByRole("searchbox", { name: "Search chats" });
  await user.type(search, "#62");
  const inbox = screen.getByRole("list", { name: "Chat inbox" });
  expect(
    within(inbox)
      .getAllByRole("listitem")
      .map((row) => row.dataset.chatId),
  ).toEqual(["middle"]);
  expect(screen.getByRole("region", { name: "Chat workspace" })).toHaveAttribute(
    "data-selected-chat-id",
    "older",
  );

  await user.keyboard("{Escape}");
  expect(search).toHaveValue("");
  expect(search).toHaveFocus();
  expect(
    within(screen.getByRole("list", { name: "Chat inbox" })).getAllByRole("listitem"),
  ).toHaveLength(3);
});

test("New Chat clears the conversation and All Projects targets the first available project", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness initialChatId="newer" />);

  await user.click(screen.getByRole("button", { name: "New Chat" }));

  expect(await screen.findByRole("heading", { name: "Start a chat" })).toBeVisible();
  expect(screen.getByText(/New chat in/)).toHaveTextContent("New chat in Atlas");
  expect(screen.getByRole("textbox", { name: "Draft for Atlas" })).toHaveValue(
    "Explain the parser",
  );
});

test("secondary click and overflow expose the same ordered Rename and Delete actions", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness />);
  const row = screen.getByRole("button", { name: "Document the importer, idle" });

  fireEvent.contextMenu(row, { clientX: 120, clientY: 160 });
  const contextMenu = await screen.findByRole("menu");
  expect(
    within(contextMenu)
      .getAllByRole("menuitem")
      .map((item) => item.textContent),
  ).toEqual(["Rename chat", "Delete chat"]);
  await user.keyboard("{Escape}");

  await user.click(
    within(row.closest("li") as HTMLLIElement).getByRole("button", {
      name: "More chat actions",
    }),
  );
  const overflowMenu = await screen.findByRole("menu");
  expect(
    within(overflowMenu)
      .getAllByRole("menuitem")
      .map((item) => item.textContent),
  ).toEqual(["Rename chat", "Delete chat"]);
});

test("rename remains presentation-only and restores focus to the row", async () => {
  const renameChat = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  render(<WorkspaceHarness onRename={renameChat} />);
  const row = screen.getByRole("button", { name: "Document the importer, idle" });

  fireEvent.contextMenu(row, { clientX: 120, clientY: 160 });
  await user.click(await screen.findByRole("menuitem", { name: "Rename chat" }));
  const title = screen.getByRole("textbox", { name: "Title" });
  await user.clear(title);
  await user.type(title, "Importer documentation");
  await user.click(screen.getByRole("button", { name: "Save" }));

  expect(renameChat).toHaveBeenCalledWith("older", "Importer documentation");
  await vi.waitFor(() => expect(row).toHaveFocus());
});

test("delete confirms local-only effects, recovers neighbor focus, and reports failure", async () => {
  const deleteChat = vi
    .fn<(chatId: string) => Promise<string | undefined>>()
    .mockResolvedValueOnce(undefined)
    .mockResolvedValueOnce("The managed worktree changed unexpectedly.");
  const user = userEvent.setup();
  render(<WorkspaceHarness onDelete={deleteChat} />);

  const older = screen.getByRole("button", { name: "Document the importer, idle" });
  fireEvent.contextMenu(older, { clientX: 120, clientY: 160 });
  await user.click(await screen.findByRole("menuitem", { name: "Delete chat" }));
  const confirmation = screen.getByRole("alertdialog", { name: /Delete.*Document the importer/ });
  expect(confirmation).toHaveTextContent("local conversation, managed worktree, and local branch");
  expect(confirmation).toHaveTextContent("won't close a pull request or delete a remote branch");
  expect(confirmation).toHaveTextContent("Any active agent or terminal will be stopped first");
  await vi.waitFor(() =>
    expect(within(confirmation).getByRole("button", { name: "Cancel" })).toHaveFocus(),
  );
  await user.click(within(confirmation).getByRole("button", { name: "Delete chat" }));
  expect(deleteChat).toHaveBeenCalledWith("older");
  await vi.waitFor(() =>
    expect(
      screen.getByRole("button", {
        name: /Keep this deliberately long chat title stable.*idle/,
      }),
    ).toHaveFocus(),
  );

  const newer = screen.getByRole("button", { name: "Repair repository indexing, idle" });
  fireEvent.contextMenu(newer, { clientX: 120, clientY: 160 });
  await user.click(await screen.findByRole("menuitem", { name: "Delete chat" }));
  await user.click(
    within(screen.getByRole("alertdialog")).getByRole("button", { name: "Delete chat" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("changed unexpectedly");
  expect(screen.getByRole("alertdialog")).toBeVisible();
});

test("preserves the thin keyboard and pointer resize seam", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness />);

  expect(screen.getByRole("button", { name: "Settings" })).toBeVisible();
  const resizeHandle = screen.getByRole("separator", { name: "Resize inbox" });
  expect(resizeHandle).toHaveAttribute("aria-valuenow", "256");
  await user.click(resizeHandle);
  await user.keyboard("{ArrowRight}");
  expect(resizeHandle).toHaveAttribute("aria-valuenow", "272");

  expect(fireEvent.pointerDown(resizeHandle, { clientX: 100, pointerId: 1 })).toBe(false);
  expect(document.documentElement).toHaveAttribute("data-inbox-sidebar-resizing");
  fireEvent.pointerMove(resizeHandle, { clientX: 111, pointerId: 1 });
  expect(resizeHandle).toHaveAttribute("aria-valuenow", "284");
  fireEvent.pointerUp(resizeHandle, { pointerId: 1 });
  expect(document.documentElement).not.toHaveAttribute("data-inbox-sidebar-resizing");
});

import { useEffect, useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { InboxSnapshot } from "../../platform/project-inbox";
import type { ConversationAdapter } from "../../platform/conversations";
import type { PromptAttachment } from "../../platform/prompt-attachments";
import { ProjectDraftController } from "./draft-controller";
import { ChatActivityController } from "./chat-activity-controller";
import { InboxWorkspace } from "./InboxWorkspace";
import { ChatSetupController } from "./setup-controller";

const readySetup = {
  phase: "succeeded" as const,
  failure: null,
  exitCode: 0,
  signal: null,
  attempt: 1,
  log: "",
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

const populatedSnapshot: InboxSnapshot = {
  projects: [
    { id: 1, name: "Atlas", availability: "available", unmergedChatCount: 2 },
    { id: 2, name: "Beacon", availability: "missing", unmergedChatCount: 1 },
    { id: 3, name: "Caldera", availability: "available", unmergedChatCount: 0 },
  ],
  drafts: [{ attachments: [], projectId: 1, prompt: "Explain the parser", updatedAtMs: 500 }],
  chats: [
    {
      id: "older",
      projectId: 1,
      projectName: "Atlas",
      title: "Document the importer",
      branchName: "docs/importer",
      pullRequestNumber: null,
      createdAtMs: 100,
      mergeState: "unmerged",
      setup: readySetup,
    },
    {
      id: "newer",
      projectId: 2,
      projectName: "Beacon",
      title: "Repair repository indexing",
      branchName: "fix/indexing",
      pullRequestNumber: 73,
      createdAtMs: 300,
      mergeState: "unmerged",
      setup: readySetup,
    },
    {
      id: "middle",
      projectId: 1,
      projectName: "Atlas",
      title: "Keep this deliberately long chat title stable while its narrow row truncates cleanly",
      branchName: "feature/stable-order",
      pullRequestNumber: 62,
      createdAtMs: 200,
      mergeState: "unmerged",
      setup: readySetup,
    },
    {
      id: "merged",
      projectId: null,
      projectName: "Removed project",
      title: "Historical result",
      branchName: "agent/history",
      pullRequestNumber: 41,
      createdAtMs: 50,
      mergeState: "merged",
      setup: readySetup,
    },
  ],
};

function WorkspaceHarness({
  initialSnapshot = populatedSnapshot,
  onCreate = vi.fn().mockResolvedValue(undefined),
  onRemove = vi.fn().mockResolvedValue(undefined),
  onRename = vi.fn().mockResolvedValue(undefined),
  onSave,
}: {
  initialSnapshot?: InboxSnapshot;
  onCreate?: (
    projectId: number,
    prompt: string,
    attachments: readonly PromptAttachment[],
  ) => Promise<string | undefined>;
  onRemove?: (projectId: number) => Promise<string | undefined>;
  onRename?: (chatId: string, title: string) => Promise<string | undefined>;
  onSave?: (projectId: number, prompt: string) => Promise<void>;
}) {
  const [snapshot, setSnapshot] = useState(initialSnapshot);
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [selectedChatId, setSelectedChatId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [drafts] = useState(() => {
    const controller = new ProjectDraftController(async (projectId, prompt) => {
      await onSave?.(projectId, prompt);
      setSnapshot((current) => ({
        ...current,
        drafts: [
          ...current.drafts.filter((draft) => draft.projectId !== projectId),
          ...(prompt ? [{ attachments: [], projectId, prompt, updatedAtMs: 700 }] : []),
        ],
      }));
    });
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

  return (
    <InboxWorkspace
      actionError={undefined}
      activities={activities}
      conversationAdapter={conversationAdapter}
      conversationRevision={0}
      drafts={drafts}
      onCancelSetup={vi.fn().mockResolvedValue(undefined)}
      onCreateChat={onCreate}
      onOpenRepository={vi.fn()}
      onOpenTerminal={vi.fn().mockResolvedValue(undefined)}
      onOpenSettings={vi.fn()}
      onRequestCodexSignIn={vi.fn()}
      onQueryChange={setQuery}
      onRemoveProject={onRemove}
      onRenameChat={onRename}
      onRetrySetup={vi.fn().mockResolvedValue(undefined)}
      onSelectChat={setSelectedChatId}
      onSelectProject={(projectId) => {
        setSelectedProjectId(projectId);
        setSelectedChatId(null);
      }}
      query={query}
      selectedChatId={selectedChatId}
      selectedProjectId={selectedProjectId}
      setups={setups}
      snapshot={snapshot}
    />
  );
}

test("exposes Settings as a quiet sidebar footer action", async () => {
  const openSettings = vi.fn();
  const user = userEvent.setup();
  render(
    <InboxWorkspace
      actionError={undefined}
      activities={new ChatActivityController()}
      conversationAdapter={conversationAdapter}
      conversationRevision={0}
      drafts={new ProjectDraftController(() => Promise.resolve())}
      onCancelSetup={vi.fn().mockResolvedValue(undefined)}
      onCreateChat={vi.fn().mockResolvedValue(undefined)}
      onOpenRepository={vi.fn()}
      onOpenTerminal={vi.fn().mockResolvedValue(undefined)}
      onOpenSettings={openSettings}
      onRequestCodexSignIn={vi.fn()}
      onQueryChange={vi.fn()}
      onRemoveProject={vi.fn().mockResolvedValue(undefined)}
      onRenameChat={vi.fn().mockResolvedValue(undefined)}
      onRetrySetup={vi.fn().mockResolvedValue(undefined)}
      onSelectChat={vi.fn()}
      onSelectProject={vi.fn()}
      query=""
      selectedChatId={null}
      selectedProjectId={null}
      setups={new ChatSetupController()}
      snapshot={{ projects: [], drafts: [], chats: [] }}
    />,
  );

  await user.click(screen.getByRole("button", { name: "Settings" }));
  expect(openSettings).toHaveBeenCalledOnce();
});

test("renders stable global rows and composes project filtering with search", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness />);

  const inbox = screen.getByRole("list", { name: "Active chats" });
  expect(
    within(inbox)
      .getAllByRole("listitem")
      .map((row) => row.dataset.chatId),
  ).toEqual(["newer", "middle", "older"]);

  await user.click(screen.getByRole("button", { name: /Atlas, available, 2 active chats/ }));
  expect(screen.getByRole("textbox", { name: "Draft for Atlas" })).toHaveValue(
    "Explain the parser",
  );
  expect(
    within(screen.getByRole("list", { name: "Active chats" })).getAllByRole("listitem"),
  ).toHaveLength(2);

  await user.type(screen.getByRole("searchbox", { name: "Search chats" }), "#62");
  expect(screen.queryByRole("textbox", { name: "Draft for Atlas" })).not.toBeInTheDocument();
  expect(screen.getByText(/deliberately long chat title/)).toBeVisible();
  expect(screen.queryByText("Document the importer")).not.toBeInTheDocument();
});

test("offers chat actions through right click and a visible menu, then renames presentation only", async () => {
  const renameChat = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  render(<WorkspaceHarness onRename={renameChat} />);

  const row = screen.getByRole("button", { name: "Document the importer, idle" });
  expect(
    within(row.closest("li") as HTMLLIElement).getByRole("button", {
      name: "More chat actions",
    }),
  ).toBeInTheDocument();

  fireEvent.contextMenu(row, { clientX: 120, clientY: 160 });
  await user.click(await screen.findByRole("menuitem", { name: "Rename chat" }));

  const title = screen.getByRole("textbox", { name: "Title" });
  expect(title).toHaveValue("Document the importer");
  await user.clear(title);
  await user.type(title, "Importer documentation");
  await user.click(screen.getByRole("button", { name: "Save" }));

  expect(renameChat).toHaveBeenCalledWith("older", "Importer documentation");
  expect(screen.queryByRole("dialog", { name: "Rename chat" })).not.toBeInTheDocument();
});

test("keeps one controlled draft per project across navigation", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness />);

  await user.click(screen.getByRole("button", { name: /Atlas, available, 2 active chats/ }));
  const draft = screen.getByRole("textbox", { name: "Draft for Atlas" });
  await user.clear(draft);
  await user.type(draft, "A replacement prompt");
  expect(draft).toHaveValue("A replacement prompt");

  await user.click(screen.getByRole("button", { name: /Caldera, available, 0 active chats/ }));
  expect(screen.getByRole("textbox", { name: "Draft for Caldera" })).toHaveValue("");
  await user.click(screen.getByRole("button", { name: /Atlas, available, 2 active chats/ }));
  expect(screen.getByRole("textbox", { name: "Draft for Atlas" })).toHaveValue(
    "A replacement prompt",
  );
});

test("scopes a failed chat submission to the project that produced it", async () => {
  const onCreate = vi
    .fn()
    .mockResolvedValueOnce(
      "Più couldn’t fetch a fresh origin/main. Check remote access and try again.",
    )
    .mockResolvedValue(undefined);
  const user = userEvent.setup();
  render(<WorkspaceHarness onCreate={onCreate} />);

  const atlasDraft = screen.getByRole("textbox", { name: "Draft for Atlas" });
  await user.clear(atlasDraft);
  await user.type(atlasDraft, "Create the Atlas chat");
  await user.click(screen.getByRole("button", { name: "Send message" }));
  expect(await screen.findByText(/couldn’t fetch a fresh origin\/main/)).toBeVisible();

  await user.click(screen.getByRole("button", { name: /Caldera, available, 0 active chats/ }));

  expect(screen.getByRole("textbox", { name: "Draft for Caldera" })).toBeVisible();
  expect(screen.queryByText(/couldn’t fetch a fresh origin\/main/)).not.toBeInTheDocument();
});

test("All Projects uses the first available project as the centered composer target", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness />);

  expect(screen.getByRole("button", { name: "All Projects, 3 projects" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  const composer = screen.getByRole("textbox", { name: "Draft for Atlas" });
  expect(composer).toHaveValue("Explain the parser");
  expect(screen.getByRole("heading", { name: "Start a chat" })).toBeVisible();
  expect(screen.getByText(/New chat in/)).toHaveTextContent("New chat in Atlas");

  await user.clear(composer);
  await user.type(composer, "Use the global inbox draft");
  expect(composer).toHaveValue("Use the global inbox draft");
});

test("the inbox sidebar resize seam is keyboard operable", async () => {
  const user = userEvent.setup();
  render(<WorkspaceHarness />);

  const resizeHandle = screen.getByRole("separator", { name: "Resize inbox" });
  expect(resizeHandle).toHaveAttribute("aria-valuenow", "256");
  await user.click(resizeHandle);
  await user.keyboard("{ArrowRight}");
  expect(resizeHandle).toHaveAttribute("aria-valuenow", "272");
  await user.keyboard("{Home}");
  expect(resizeHandle).toHaveAttribute("aria-valuenow", "208");
});

test("the inbox sidebar drag width stays on the four-pixel grid", () => {
  render(<WorkspaceHarness />);

  const resizeHandle = screen.getByRole("separator", { name: "Resize inbox" });
  expect(fireEvent.pointerDown(resizeHandle, { clientX: 100, pointerId: 1 })).toBe(false);
  expect(document.documentElement).toHaveAttribute("data-inbox-sidebar-resizing");
  fireEvent.pointerMove(resizeHandle, { clientX: 111, pointerId: 1 });

  expect(resizeHandle).toHaveAttribute("aria-valuenow", "268");

  fireEvent.pointerUp(resizeHandle, { pointerId: 1 });
  expect(document.documentElement).not.toHaveAttribute("data-inbox-sidebar-resizing");
});

test("explains unavailable projects and enforces removal rules", async () => {
  const removeProject = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  render(<WorkspaceHarness onRemove={removeProject} />);

  const blockedTrigger = screen.getByRole("button", { name: "Project actions for Atlas" });
  await user.click(blockedTrigger);
  const blockedRemoval = await screen.findByRole("menuitem", { name: "Remove project" });
  expect(blockedRemoval).toHaveAttribute("aria-disabled", "true");
  expect(blockedRemoval).toHaveAccessibleDescription("Merge active chats before removing Atlas.");
  await user.click(blockedRemoval);
  expect(removeProject).not.toHaveBeenCalled();

  await user.keyboard("{Escape}");
  await user.click(screen.getByRole("button", { name: /Beacon, repository unavailable/ }));
  expect(screen.getByRole("heading", { name: "Beacon is unavailable" })).toBeVisible();
  expect(screen.queryByRole("textbox", { name: /Beacon/ })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Send message/ })).not.toBeInTheDocument();

  const trigger = screen.getByRole("button", { name: "Project actions for Caldera" });
  await user.click(trigger);
  await user.click(await screen.findByRole("menuitem", { name: "Remove project" }));
  const confirmation = screen.getByRole("alertdialog", { name: "Remove Caldera?" });
  expect(confirmation).toHaveTextContent("repository on disk won't be changed");
  await vi.waitFor(() =>
    expect(within(confirmation).getByRole("button", { name: "Cancel" })).toHaveFocus(),
  );
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  expect(trigger).toHaveFocus();

  await user.click(trigger);
  await user.click(await screen.findByRole("menuitem", { name: "Remove project" }));
  const reopenedConfirmation = screen.getByRole("alertdialog", { name: "Remove Caldera?" });
  await user.click(within(reopenedConfirmation).getByRole("button", { name: "Remove project" }));
  expect(removeProject).toHaveBeenCalledWith(3);
});

test("an unavailable project presents its retained draft read-only without send affordances", async () => {
  const retainedSnapshot: InboxSnapshot = {
    ...populatedSnapshot,
    drafts: [
      ...populatedSnapshot.drafts,
      {
        attachments: [],
        projectId: 2,
        prompt: "Keep the unavailable repository context",
        updatedAtMs: 600,
      },
    ],
  };
  const user = userEvent.setup();
  render(<WorkspaceHarness initialSnapshot={retainedSnapshot} />);

  await user.click(screen.getByRole("button", { name: /Beacon, repository unavailable/ }));

  const retainedDraft = screen.getByRole("textbox", { name: "Retained draft for Beacon" });
  expect(retainedDraft).toHaveValue("Keep the unavailable repository context");
  expect(retainedDraft).toHaveAttribute("readonly");
  expect(retainedDraft).not.toHaveAttribute("placeholder");
  expect(screen.queryByRole("button", { name: /Send message/ })).not.toBeInTheDocument();
});

test("removal discloses draft deletion and keeps the modal open on failure", async () => {
  const removeProject = vi.fn().mockResolvedValue("Couldn't remove that project. Try again.");
  const user = userEvent.setup();
  render(<WorkspaceHarness onRemove={removeProject} />);

  await user.click(screen.getByRole("button", { name: /Caldera, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Caldera" }), "Unsent work");
  await user.click(screen.getByRole("button", { name: "Project actions for Caldera" }));
  await user.click(await screen.findByRole("menuitem", { name: "Remove project" }));
  const dialog = screen.getByRole("alertdialog", { name: "Remove Caldera?" });
  expect(dialog).toHaveTextContent("unsent draft will be deleted");

  await user.click(within(dialog).getByRole("button", { name: "Remove project" }));

  expect(await within(dialog).findByRole("alert")).toHaveTextContent("Couldn't remove");
  expect(dialog).toBeVisible();
});

test("attachment-only drafts stay visible and are disclosed before project removal", async () => {
  const user = userEvent.setup();
  const attachmentOnlySnapshot: InboxSnapshot = {
    ...populatedSnapshot,
    drafts: [
      ...populatedSnapshot.drafts,
      {
        attachments: [
          {
            content: "Release notes",
            id: "notes-attachment",
            kind: "text",
            mimeType: "text/plain",
            name: "notes.txt",
            sizeBytes: 13,
          },
        ],
        projectId: 3,
        prompt: "",
        updatedAtMs: 700,
      },
    ],
  };
  render(<WorkspaceHarness initialSnapshot={attachmentOnlySnapshot} />);

  const draftsList = screen.getByRole("list", { name: "Unsent drafts" });
  expect(within(draftsList).getByText("Attached notes.txt")).toBeVisible();

  await user.click(screen.getByRole("button", { name: "Project actions for Caldera" }));
  await user.click(await screen.findByRole("menuitem", { name: "Remove project" }));

  expect(screen.getByRole("alertdialog", { name: "Remove Caldera?" })).toHaveTextContent(
    "unsent draft will be deleted",
  );
});

test("a failed draft stays visible in All Projects and can be retried", async () => {
  const save = vi
    .fn<(projectId: number, prompt: string) => Promise<void>>()
    .mockRejectedValueOnce(new Error("database offline"))
    .mockResolvedValueOnce();
  const user = userEvent.setup();
  render(<WorkspaceHarness onSave={save} />);

  await user.click(screen.getByRole("button", { name: /Caldera, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Caldera" }), "Keep this work");
  expect(await screen.findByText(/Couldn't save this draft/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "All Projects, 3 projects" }));

  const draftsList = screen.getByRole("list", { name: "Unsent drafts" });
  expect(within(draftsList).getByText("Keep this work")).toBeVisible();
  await user.click(within(draftsList).getByRole("button", { name: "Retry" }));
  await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(within(draftsList).queryByText("Not saved")).not.toBeInTheDocument();
});

test("removal keeps focus contained while the host response is pending", async () => {
  let finishRemoval: ((value: string | undefined) => void) | undefined;
  const removeProject = vi.fn(
    () =>
      new Promise<string | undefined>((resolve) => {
        finishRemoval = resolve;
      }),
  );
  const user = userEvent.setup();
  render(<WorkspaceHarness onRemove={removeProject} />);

  await user.click(screen.getByRole("button", { name: "Project actions for Caldera" }));
  await user.click(await screen.findByRole("menuitem", { name: "Remove project" }));
  const dialog = screen.getByRole("alertdialog", { name: "Remove Caldera?" });
  await user.click(within(dialog).getByRole("button", { name: "Remove project" }));
  expect(within(dialog).getByRole("button", { name: "Removing" })).toBeDisabled();

  await user.keyboard("{Tab}{Tab}");
  expect(dialog).toContainElement(document.activeElement as HTMLElement);
  await user.keyboard("{Escape}");
  expect(dialog).toBeVisible();

  finishRemoval?.(undefined);
  await vi.waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
});

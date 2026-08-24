import { useEffect, useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { InboxSnapshot } from "../../platform/project-inbox";
import { ProjectDraftController } from "./draft-controller";
import { InboxWorkspace } from "./InboxWorkspace";

const populatedSnapshot: InboxSnapshot = {
  projects: [
    { id: 1, name: "Atlas", availability: "available", unmergedChatCount: 2 },
    { id: 2, name: "Beacon", availability: "missing", unmergedChatCount: 1 },
    { id: 3, name: "Caldera", availability: "available", unmergedChatCount: 0 },
  ],
  drafts: [{ projectId: 1, prompt: "Explain the parser", updatedAtMs: 500 }],
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
    },
  ],
};

function WorkspaceHarness({
  initialSnapshot = populatedSnapshot,
  onRemove = vi.fn().mockResolvedValue(undefined),
  onSave,
}: {
  initialSnapshot?: InboxSnapshot;
  onRemove?: (projectId: number) => Promise<string | undefined>;
  onSave?: (projectId: number, prompt: string) => Promise<void>;
}) {
  const [snapshot, setSnapshot] = useState(initialSnapshot);
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [drafts] = useState(() => {
    const controller = new ProjectDraftController(async (projectId, prompt) => {
      await onSave?.(projectId, prompt);
      setSnapshot((current) => ({
        ...current,
        drafts: [
          ...current.drafts.filter((draft) => draft.projectId !== projectId),
          ...(prompt ? [{ projectId, prompt, updatedAtMs: 700 }] : []),
        ],
      }));
    });
    controller.reconcile(initialSnapshot);
    return controller;
  });
  useEffect(() => drafts.reconcile(snapshot), [drafts, snapshot]);

  return (
    <InboxWorkspace
      actionError={undefined}
      drafts={drafts}
      onOpenRepository={vi.fn()}
      onQueryChange={setQuery}
      onRemoveProject={onRemove}
      onSelectProject={setSelectedProjectId}
      query={query}
      selectedProjectId={selectedProjectId}
      snapshot={snapshot}
    />
  );
}

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
  fireEvent.pointerDown(resizeHandle, { clientX: 100, pointerId: 1 });
  fireEvent.pointerMove(resizeHandle, { clientX: 111, pointerId: 1 });

  expect(resizeHandle).toHaveAttribute("aria-valuenow", "268");
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
      { projectId: 2, prompt: "Keep the unavailable repository context", updatedAtMs: 600 },
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

test("a failed draft stays visible in All Projects and can be retried", async () => {
  const save = vi
    .fn<(projectId: number, prompt: string) => Promise<void>>()
    .mockRejectedValueOnce(new Error("database offline"))
    .mockResolvedValueOnce();
  const user = userEvent.setup();
  render(<WorkspaceHarness onSave={save} />);

  await user.click(screen.getByRole("button", { name: /Caldera, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Caldera" }), "Keep this work");
  expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't save this draft");
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

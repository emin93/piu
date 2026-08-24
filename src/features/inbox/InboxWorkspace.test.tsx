import { useState } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { InboxSnapshot } from "../../platform/project-inbox";
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
  onRemove = vi.fn().mockResolvedValue(undefined),
}: {
  onRemove?: (projectId: number) => Promise<string | undefined>;
}) {
  const [snapshot, setSnapshot] = useState(populatedSnapshot);
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [query, setQuery] = useState("");

  return (
    <InboxWorkspace
      actionError={undefined}
      draftStatus={{ state: "saved" }}
      onDraftChange={(projectId, prompt) => {
        setSnapshot((current) => ({
          ...current,
          drafts: [
            ...current.drafts.filter((draft) => draft.projectId !== projectId),
            { projectId, prompt, updatedAtMs: 700 },
          ],
        }));
      }}
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

test("explains unavailable projects and enforces removal rules", async () => {
  const removeProject = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  render(<WorkspaceHarness onRemove={removeProject} />);

  const blockedRemoval = screen.getByRole("button", { name: "Remove Atlas" });
  expect(blockedRemoval).toHaveAttribute("aria-disabled", "true");
  expect(blockedRemoval).toHaveAccessibleDescription("Merge active chats before removing Atlas.");
  expect(screen.getByRole("button", { name: "Remove Beacon" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  expect(screen.getByText("Repository unavailable")).toBeVisible();
  await user.click(blockedRemoval);
  expect(blockedRemoval).toHaveFocus();
  expect(removeProject).not.toHaveBeenCalled();

  const trigger = screen.getByRole("button", { name: "Remove Caldera" });
  await user.click(trigger);
  const confirmation = screen.getByRole("dialog", { name: "Remove Caldera?" });
  expect(confirmation).toHaveTextContent("repository on disk won’t be changed");
  expect(within(confirmation).getByRole("button", { name: "Cancel" })).toHaveFocus();
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(trigger).toHaveFocus();

  await user.click(trigger);
  const reopenedConfirmation = screen.getByRole("dialog", { name: "Remove Caldera?" });
  await user.click(within(reopenedConfirmation).getByRole("button", { name: "Remove project" }));
  expect(removeProject).toHaveBeenCalledWith(3);
});

test("removal discloses draft deletion and keeps the modal open on failure", async () => {
  const removeProject = vi.fn().mockResolvedValue("Couldn't remove that project. Try again.");
  const user = userEvent.setup();
  render(<WorkspaceHarness onRemove={removeProject} />);

  await user.click(screen.getByRole("button", { name: /Caldera, available/ }));
  await user.type(screen.getByRole("textbox", { name: "Draft for Caldera" }), "Unsent work");
  await user.click(screen.getByRole("button", { name: "Remove Caldera" }));
  const dialog = screen.getByRole("dialog", { name: "Remove Caldera?" });
  expect(dialog).toHaveTextContent("unsent draft will be deleted");

  await user.click(within(dialog).getByRole("button", { name: "Remove project" }));

  expect(await within(dialog).findByRole("alert")).toHaveTextContent("Couldn't remove");
  expect(dialog).toBeVisible();
});

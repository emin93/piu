import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { ProjectSummary } from "@/platform/project-inbox";

import { ChatComposer } from "./ChatComposer";
import { ProjectDraftController } from "./draft-controller";

const project: ProjectSummary = {
  id: 7,
  name: "Atlas",
  availability: "available",
  unmergedChatCount: 0,
};

test("moves the same focused composer from centered to docked without losing its draft", async () => {
  const drafts = new ProjectDraftController(vi.fn().mockResolvedValue(undefined));
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  const { rerender } = render(
    <ChatComposer drafts={drafts} layout="centered" onSubmit={onSubmit} project={project} />,
  );

  const textarea = screen.getByRole("textbox", { name: "Draft for Atlas" });
  await waitFor(() => expect(textarea).toHaveFocus());
  await user.type(textarea, "Preserve this exact draft and focus");
  const stage = textarea.closest("section");
  const composer = textarea.closest("form");
  expect(stage).toHaveAttribute("data-composer-layout", "centered");
  expect(composer).toHaveAttribute("data-composer-layout", "centered");
  await user.keyboard("{Meta>}{Enter}{/Meta}");
  expect(onSubmit).not.toHaveBeenCalled();
  expect(textarea).toHaveValue("Preserve this exact draft and focus\n");

  rerender(<ChatComposer drafts={drafts} layout="docked" onSubmit={onSubmit} project={project} />);

  const dockedTextarea = screen.getByRole("textbox", { name: "Draft for Atlas" });
  expect(dockedTextarea).toBe(textarea);
  expect(dockedTextarea).toHaveValue("Preserve this exact draft and focus\n");
  expect(dockedTextarea).toHaveFocus();
  expect(stage).toHaveAttribute("data-composer-layout", "docked");
  expect(composer).toHaveAttribute("data-composer-layout", "docked");
});

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

const attachmentPlatform = vi.hoisted(() => ({
  attachmentErrorMessage: vi.fn(() => "Couldn’t attach"),
  selectPromptAttachments: vi.fn(),
}));

vi.mock("@/platform/prompt-attachments", () => attachmentPlatform);

import { PromptAttachmentButton, PromptAttachmentTray } from "./PromptAttachments";

const attachment = {
  content: "hello",
  id: "attachment-1",
  kind: "text" as const,
  mimeType: "text/plain",
  name: "notes.txt",
  sizeBytes: 5,
};

test("selected files are previewed and removable before send", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  attachmentPlatform.selectPromptAttachments.mockResolvedValue({
    attachments: [attachment],
    outcome: "selected",
  });
  const { rerender } = render(
    <PromptAttachmentButton attachments={[]} onChange={onChange} onError={vi.fn()} />,
  );

  await user.click(screen.getByRole("button", { name: "Attach files" }));
  expect(onChange).toHaveBeenCalledWith([attachment]);

  const onRemove = vi.fn();
  rerender(<PromptAttachmentTray attachments={[attachment]} onRemove={onRemove} />);
  expect(screen.getByRole("list", { name: "Attached files" })).toHaveTextContent("notes.txt");
  await user.click(screen.getByRole("button", { name: "Remove notes.txt" }));
  expect(onRemove).toHaveBeenCalledWith("attachment-1");
});

test("picker cancellation does not alter the retained draft", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  attachmentPlatform.selectPromptAttachments.mockResolvedValue({ outcome: "cancelled" });
  render(
    <PromptAttachmentButton attachments={[attachment]} onChange={onChange} onError={vi.fn()} />,
  );

  await user.click(screen.getByRole("button", { name: "Attach files" }));
  expect(onChange).not.toHaveBeenCalled();
});

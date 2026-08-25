import { beforeEach, expect, test, vi } from "vitest";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
const tauri = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("@tauri-apps/api/core", () => tauri);

import { selectPromptAttachments } from "./prompt-attachments";

beforeEach(() => {
  dialog.open.mockReset();
  tauri.invoke.mockReset();
});

test("cancelling the native file picker is a typed no-op", async () => {
  dialog.open.mockResolvedValue(null);

  await expect(selectPromptAttachments([])).resolves.toEqual({ outcome: "cancelled" });
  expect(tauri.invoke).not.toHaveBeenCalled();
});

test("individual file paths are prepared by the native immutable attachment boundary", async () => {
  dialog.open.mockResolvedValue(["/tmp/notes.txt", "/tmp/view.png"]);
  tauri.invoke.mockResolvedValue([
    {
      content: "hello",
      id: "text-1",
      kind: "text",
      mimeType: "text/plain",
      name: "notes.txt",
      sizeBytes: 5,
    },
  ]);

  await expect(selectPromptAttachments([])).resolves.toMatchObject({
    outcome: "selected",
    attachments: [{ id: "text-1" }],
  });
  expect(dialog.open).toHaveBeenCalledWith({
    directory: false,
    multiple: true,
    title: "Attach Files",
  });
  expect(tauri.invoke).toHaveBeenCalledWith("prepare_prompt_attachments", {
    request: { accepted: [], paths: ["/tmp/notes.txt", "/tmp/view.png"] },
  });
});

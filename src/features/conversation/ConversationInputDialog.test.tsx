import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import { ConversationInputDialog } from "./ConversationInputDialog";

test("answers a Pi selection with the exact option value", async () => {
  const user = userEvent.setup();
  const onAnswer = vi.fn().mockResolvedValue(undefined);
  render(
    <ConversationInputDialog
      onAnswer={onAnswer}
      request={{
        id: "choice-1",
        kind: "select",
        message: null,
        options: ["Keep both", "Replace"],
        placeholder: null,
        prefill: null,
        title: "Choose a strategy",
      }}
    />,
  );

  await user.click(screen.getByRole("button", { name: "Keep both" }));

  expect(onAnswer).toHaveBeenCalledWith({ kind: "value", value: "Keep both" });
});

test("preserves editor prefill and exposes explicit cancellation", async () => {
  const user = userEvent.setup();
  const onAnswer = vi.fn().mockResolvedValue(undefined);
  const { rerender } = render(
    <ConversationInputDialog
      onAnswer={onAnswer}
      request={{
        id: "editor-1",
        kind: "editor",
        message: null,
        options: [],
        placeholder: null,
        prefill: "Keep the public API",
        title: "Explain the choice",
      }}
    />,
  );
  const editor = screen.getByRole("textbox", { name: "Explain the choice" });
  await user.clear(editor);
  await user.type(editor, "Keep both implementations");
  await user.click(screen.getByRole("button", { name: "Submit" }));
  expect(onAnswer).toHaveBeenLastCalledWith({
    kind: "value",
    value: "Keep both implementations",
  });

  onAnswer.mockClear();
  rerender(
    <ConversationInputDialog
      onAnswer={onAnswer}
      request={{
        id: "confirm-2",
        kind: "confirm",
        message: "Continue with the change?",
        options: [],
        placeholder: null,
        prefill: null,
        title: "Confirm change",
      }}
    />,
  );
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onAnswer).toHaveBeenCalledWith({ kind: "cancelled" });
});

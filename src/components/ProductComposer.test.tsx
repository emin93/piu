import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { expect, test, vi } from "vitest";

import { ProductComposer } from "./ProductComposer";

test("the product composer exposes one layout, status, and action contract", async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn();

  function ControlledComposer({
    layout,
    submitOnMetaEnter,
  }: {
    layout: "centered" | "docked";
    submitOnMetaEnter?: boolean;
  }) {
    const [value, setValue] = useState("");
    return (
      <ProductComposer
        actions={<button type="submit">Send</button>}
        ariaLabel="Message Più"
        layout={layout}
        onSubmit={onSubmit}
        onValueChange={setValue}
        placeholder="Describe the change"
        status={<span>Saved locally</span>}
        submitOnMetaEnter={submitOnMetaEnter}
        value={value}
      />
    );
  }

  const { rerender } = render(<ControlledComposer layout="centered" />);
  const input = screen.getByRole("textbox", { name: "Message Più" });
  const composer = input.closest("form");

  expect(composer).toHaveAttribute("data-composer-layout", "centered");
  expect(screen.getByText("Saved locally")).toBeVisible();
  await user.type(input, "Keep one composer");
  await user.keyboard("{Meta>}{Enter}{/Meta}");
  expect(onSubmit).not.toHaveBeenCalled();

  rerender(<ControlledComposer layout="docked" submitOnMetaEnter />);
  expect(composer).toHaveAttribute("data-composer-layout", "docked");
  await user.keyboard("{Meta>}{Enter}{/Meta}");
  expect(onSubmit).toHaveBeenCalledOnce();
  expect(screen.getByRole("textbox", { name: "Message Più" })).toBe(input);
});

test("an error is automatically included in the input description", () => {
  render(
    <ProductComposer
      actions={<button type="submit">Send</button>}
      ariaDescribedBy="draft-status"
      ariaLabel="Draft for Atlas"
      error={{
        action: <button type="button">Retry save</button>,
        message: "The draft could not be saved.",
      }}
      layout="centered"
      onValueChange={vi.fn()}
      status={<span id="draft-status">Not saved</span>}
      value="Keep this draft"
    />,
  );

  expect(screen.getByRole("textbox", { name: "Draft for Atlas" })).toHaveAccessibleDescription(
    "Not saved The draft could not be saved.",
  );
  expect(screen.getByRole("alert")).toHaveTextContent("The draft could not be saved.Retry save");
});

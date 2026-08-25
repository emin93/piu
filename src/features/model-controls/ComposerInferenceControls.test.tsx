import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { expect, test, vi } from "vitest";

import type { ModelControlsStoreSnapshot } from "./model-controls-controller";
import { ComposerInferenceControls } from "./ComposerInferenceControls";

const qwenRoute = { modelId: "qwen3.8-27b", provider: "piu-local" };
const codexRoute = { modelId: "gpt-5.6-sol", provider: "openai-codex" };
const ready: ModelControlsStoreSnapshot = {
  controls: {
    appliesAfterCurrentStep: false,
    efforts: ["low", "medium", "xhigh"],
    routes: [
      { acceptsImages: true, id: qwenRoute, name: "Qwen 3.8 27B" },
      { acceptsImages: true, id: codexRoute, name: "GPT-5.6 Sol" },
    ],
    selectedEffort: "medium",
    selectedRoute: qwenRoute,
  },
  error: null,
  pending: null,
  phase: "ready",
};

test("the composer exposes every effective route and only Pi's reasoning efforts", async () => {
  const user = userEvent.setup();
  const selectRoute = vi.fn();
  const selectEffort = vi.fn();
  render(
    <ComposerInferenceControls
      onSelectEffort={selectEffort}
      onSelectRoute={selectRoute}
      snapshot={ready}
    />,
  );

  const modelTrigger = screen.getByRole("button", { name: "Model: Qwen 3.8 27B" });
  const effortTrigger = screen.getByRole("button", { name: "Reasoning effort: Medium" });
  expect(modelTrigger).toHaveTextContent("Qwen 3.8 27B");
  expect(effortTrigger).toHaveTextContent("Medium");
  expect(screen.queryByText(/default/i)).not.toBeInTheDocument();

  await user.click(modelTrigger);
  expect(await screen.findByRole("menuitemradio", { name: "Qwen 3.8 27B" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await user.click(screen.getByRole("menuitemradio", { name: "GPT-5.6 Sol" }));
  expect(selectRoute).toHaveBeenCalledWith(codexRoute);

  await user.click(effortTrigger);
  expect(await screen.findByRole("menuitemradio", { name: "Low" })).toBeVisible();
  expect(screen.getByRole("menuitemradio", { name: "Medium" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(screen.getByRole("menuitemradio", { name: "Extra High" })).toBeVisible();
  expect(screen.queryByRole("menuitemradio", { name: "High" })).not.toBeInTheDocument();
  expect(screen.queryByRole("menuitemradio", { name: "Maximum" })).not.toBeInTheDocument();

  await user.click(screen.getByRole("menuitemradio", { name: "Extra High" }));
  expect(selectEffort).toHaveBeenCalledWith("xhigh");
});

test("pending controls expose the requested value without claiming it already applies", () => {
  render(
    <ComposerInferenceControls
      onSelectEffort={vi.fn()}
      onSelectRoute={vi.fn()}
      snapshot={{
        ...ready,
        pending: { kind: "route", route: codexRoute },
        phase: "changing",
      }}
    />,
  );

  expect(screen.getByRole("button", { name: "Model: GPT-5.6 Sol, switching" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Reasoning effort: Medium" })).toBeDisabled();
});

test("keyboard users can inspect routes and return focus to the trigger", async () => {
  const user = userEvent.setup();
  render(
    <ComposerInferenceControls onSelectEffort={vi.fn()} onSelectRoute={vi.fn()} snapshot={ready} />,
  );
  const trigger = screen.getByRole("button", { name: "Model: Qwen 3.8 27B" });
  trigger.focus();

  await user.keyboard("{Enter}");
  expect(await screen.findByRole("menuitemradio", { name: "Qwen 3.8 27B" })).toHaveFocus();
  await user.keyboard("{Escape}");

  expect(trigger).toHaveFocus();
  expect(screen.queryByRole("menuitemradio", { name: "Qwen 3.8 27B" })).not.toBeInTheDocument();
});

test("a single effective effort does not add a useless picker", () => {
  render(
    <ComposerInferenceControls
      onSelectEffort={vi.fn()}
      onSelectRoute={vi.fn()}
      snapshot={{
        ...ready,
        controls: {
          ...ready.controls!,
          efforts: ["max"],
          selectedEffort: "max",
        },
      }}
    />,
  );

  expect(screen.getByRole("button", { name: "Model: Qwen 3.8 27B" })).toBeVisible();
  expect(screen.queryByRole("button", { name: /reasoning effort/i })).not.toBeInTheDocument();
});

test("streaming parent updates do not render the memoized controls again", async () => {
  const user = userEvent.setup();
  let snapshotReads = 0;
  const tracked: ModelControlsStoreSnapshot = Object.defineProperty({ ...ready }, "controls", {
    enumerable: true,
    get() {
      snapshotReads += 1;
      return ready.controls;
    },
  });
  const selectEffort = vi.fn();
  const selectRoute = vi.fn();

  function StreamingHarness() {
    const [text, setText] = useState("Checking");
    return (
      <>
        <span>{text}</span>
        <button onClick={() => setText("Checking the bundle")}>Stream token</button>
        <ComposerInferenceControls
          onSelectEffort={selectEffort}
          onSelectRoute={selectRoute}
          snapshot={tracked}
        />
      </>
    );
  }

  render(<StreamingHarness />);
  const readsAfterMount = snapshotReads;
  await user.click(screen.getByRole("button", { name: "Stream token" }));

  expect(screen.getByText("Checking the bundle")).toBeVisible();
  expect(snapshotReads).toBe(readsAfterMount);
});

import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { ChatSummary } from "@/platform/project-inbox";
import { ChatSetupPanel } from "./ChatSetupPanel";
import { ChatSetupController } from "./setup-controller";

const chat: ChatSummary = {
  id: "chat-1",
  projectId: 1,
  projectName: "Atlas",
  title: "Prepare Atlas",
  branchName: "agent/chat-1-prepare-atlas",
  pullRequestNumber: null,
  createdAtMs: 1,
  mergeState: "unmerged",
  setup: {
    phase: "running",
    failure: null,
    exitCode: null,
    signal: null,
    attempt: 1,
    log: "Installing packages\n",
  },
};

function renderPanel(controller: ChatSetupController, overrides = {}) {
  return render(
    <ChatSetupPanel
      chat={chat}
      onCancel={vi.fn().mockResolvedValue(undefined)}
      onOpenTerminal={vi.fn().mockResolvedValue(undefined)}
      onRetry={vi.fn().mockResolvedValue(undefined)}
      setups={controller}
      {...overrides}
    />,
  );
}

test("streams setup output into one focused status view", () => {
  const controller = new ChatSetupController();
  controller.apply({ chatId: chat.id, setup: chat.setup });
  renderPanel(controller);

  expect(screen.getByRole("heading", { name: "Setting up worktree" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Setting up worktree" })).toHaveAttribute(
    "aria-busy",
    "true",
  );
  expect(screen.getByRole("status")).toHaveAttribute("aria-live", "polite");
  expect(screen.getByRole("button", { name: "Cancel setup" })).toBeVisible();
  expect(screen.getByLabelText("Setup output")).toHaveTextContent("Installing packages");

  act(() => {
    controller.apply({
      chatId: chat.id,
      setup: { ...chat.setup, log: "Installing packages\nGenerating files\n" },
    });
  });
  expect(screen.getByLabelText("Setup output")).toHaveTextContent("Generating files");
});

test("a failed setup exposes retry and terminal recovery without a path", async () => {
  const controller = new ChatSetupController();
  controller.apply({
    chatId: chat.id,
    setup: {
      ...chat.setup,
      phase: "failed",
      failure: "exit",
      exitCode: 17,
      log: "dependency install failed\n",
    },
  });
  const retry = vi.fn().mockResolvedValue(undefined);
  const openTerminal = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  renderPanel(controller, { onRetry: retry, onOpenTerminal: openTerminal });

  expect(screen.getByRole("heading", { name: "Setup failed" })).toBeVisible();
  expect(screen.getByText(/exited with code 17/)).toBeVisible();
  expect(document.body).not.toHaveTextContent("/private/");
  await user.click(screen.getByRole("button", { name: "Retry setup" }));
  await user.click(screen.getByRole("button", { name: "Open Terminal" }));

  expect(retry).toHaveBeenCalledWith(chat.id);
  expect(openTerminal).toHaveBeenCalledWith(chat.id);
});

test("a cancelled setup keeps the same explicit recovery actions", () => {
  const controller = new ChatSetupController();
  controller.apply({
    chatId: chat.id,
    setup: { ...chat.setup, phase: "cancelled" },
  });
  renderPanel(controller);

  expect(screen.getByText(/worktree is still available/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Retry setup" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Open Terminal" })).toBeVisible();
});

test.each([
  ["failed", "exit", "Setup failed"],
  ["cancelled", null, "Setup failed"],
] as const)(
  "a %s transition announces completion and moves focus to the stable status",
  (phase, failure, heading) => {
    const controller = new ChatSetupController();
    controller.apply({ chatId: chat.id, setup: chat.setup });
    renderPanel(controller);

    act(() => {
      controller.apply({
        chatId: chat.id,
        setup: {
          ...chat.setup,
          phase,
          failure,
          exitCode: phase === "failed" ? 17 : null,
        },
      });
    });

    const statusHeading = screen.getByRole("heading", { name: heading });
    expect(statusHeading).toHaveFocus();
    expect(screen.getByRole("status")).toHaveTextContent(heading);
    expect(screen.getByRole("region", { name: heading })).toHaveAttribute("aria-busy", "false");
  },
);

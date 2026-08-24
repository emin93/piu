import { expect, test, vi } from "vitest";

import type { ChatSetupSummary } from "@/platform/chat-workspaces";
import type { InboxSnapshot } from "@/platform/project-inbox";
import { ChatSetupController } from "./setup-controller";

const running = {
  phase: "running" as const,
  failure: null,
  exitCode: null,
  signal: null,
  attempt: 1,
  log: "installing\n",
};

function snapshot(setup: ChatSetupSummary = running): InboxSnapshot {
  return {
    projects: [],
    drafts: [],
    chats: [
      {
        id: "chat-1",
        projectId: null,
        projectName: "Atlas",
        title: "Setup Atlas",
        branchName: "agent/chat-1-setup-atlas",
        pullRequestNumber: null,
        createdAtMs: 1,
        mergeState: "unmerged",
        setup,
      },
    ],
  };
}

test("setup updates notify only the matching chat subscription", () => {
  const controller = new ChatSetupController();
  const first = vi.fn();
  const second = vi.fn();
  controller.subscribe("chat-1", first);
  controller.subscribe("chat-2", second);
  controller.reconcile(snapshot());

  controller.apply({
    chatId: "chat-1",
    setup: { ...running, log: "installing\ndone\n" },
  });

  expect(first).toHaveBeenCalledTimes(2);
  expect(second).not.toHaveBeenCalled();
});

test("a stale command response cannot replace newer streamed setup state", () => {
  const controller = new ChatSetupController();
  controller.apply({
    chatId: "chat-1",
    setup: {
      ...running,
      phase: "succeeded",
      log: "installing\ndone\n",
    },
  });

  controller.reconcile(snapshot({ ...running, phase: "pending", attempt: 0, log: "" }));

  expect(controller.get("chat-1")).toMatchObject({
    phase: "succeeded",
    attempt: 1,
    log: "installing\ndone\n",
  });
});

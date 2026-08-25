import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { AgentEnvironmentSnapshot } from "@/generated/AgentEnvironmentSnapshot";
import type { AgentEnvironmentAdapter } from "@/platform/agent-environment";
import type { ProjectSummary } from "@/platform/project-inbox";

import SettingsSurface from "./SettingsSurface";

vi.mock("../../model-resources/ModelResourcePanel", () => ({
  ModelResourcePanel: () => <div data-testid="managed-local-model">Managed local model</div>,
}));

const project: ProjectSummary = {
  availability: "available",
  id: 7,
  name: "Atlas",
  unmergedChatCount: 1,
};

function environment(enabled: boolean): AgentEnvironmentSnapshot {
  return {
    diagnostics: [],
    modelControls: {
      appliesAfterCurrentStep: false,
      efforts: ["low", "medium", "xhigh"],
      routes: [
        {
          acceptsImages: false,
          id: { modelId: "qwen", provider: "local" },
          name: "Qwen 3.8 27B",
        },
      ],
      selectedEffort: "medium",
      selectedRoute: { modelId: "qwen", provider: "local" },
    },
    modelRoutes: [
      {
        acceptsImages: false,
        enabled: true,
        id: { modelId: "qwen", provider: "local" },
        name: "Qwen 3.8 27B",
        thinkingLevels: ["low", "medium", "xhigh"],
      },
    ],
    resources: {
      extensions: [],
      packages: [],
      skills: [
        {
          enabled,
          id: "project://skills/review",
          name: "Review",
          origin: "topLevel",
          source: "project",
        },
      ],
    },
  };
}

test("loads the selected project and applies resource changes with safe-step feedback", async () => {
  const adapter: AgentEnvironmentAdapter = {
    get: vi.fn().mockResolvedValueOnce(environment(true)).mockResolvedValueOnce(environment(false)),
    setEnabled: vi.fn().mockResolvedValue({
      deferredChatCount: 1,
      enabled: false,
      resource: { kind: "skill", id: "project://skills/review" },
      scope: "project",
      status: "deferred",
    }),
  };
  const user = userEvent.setup();

  render(<SettingsSurface agentEnvironmentAdapter={adapter} project={project} />);

  expect(screen.getByRole("heading", { name: "Models & Resources" })).toBeVisible();
  expect(await screen.findByText("Project · Atlas")).toBeVisible();
  await user.click(screen.getByRole("switch", { name: "Review" }));

  await waitFor(() =>
    expect(adapter.setEnabled).toHaveBeenCalledWith(
      7,
      "project",
      { kind: "skill", id: "project://skills/review" },
      false,
    ),
  );
  expect(await screen.findByRole("status")).toHaveTextContent(
    "1 active chat will use this change after the current step.",
  );
  expect(screen.getByRole("switch", { name: "Review" })).not.toBeChecked();
  expect(adapter.get).toHaveBeenCalledTimes(2);
});

test("keeps the managed model available when no repository is open", () => {
  const adapter: AgentEnvironmentAdapter = { get: vi.fn(), setEnabled: vi.fn() };

  render(<SettingsSurface agentEnvironmentAdapter={adapter} />);

  expect(screen.getByRole("alert")).toHaveTextContent(
    "Open a repository to inspect its models and resources.",
  );
  expect(screen.getByTestId("managed-local-model")).toBeVisible();
  expect(adapter.get).not.toHaveBeenCalled();
});

test("recovers a failed Pi environment inspection in place", async () => {
  const adapter: AgentEnvironmentAdapter = {
    get: vi
      .fn()
      .mockRejectedValueOnce(new Error("Pi inspection failed."))
      .mockResolvedValueOnce(environment(true)),
    setEnabled: vi.fn(),
  };
  const user = userEvent.setup();

  render(<SettingsSurface agentEnvironmentAdapter={adapter} project={project} />);

  expect(await screen.findByRole("alert")).toHaveTextContent("Pi inspection failed.");
  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByRole("switch", { name: "Review" })).toBeVisible();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

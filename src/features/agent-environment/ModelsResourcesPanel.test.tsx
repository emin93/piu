import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { AgentEnvironmentSnapshot } from "@/generated/AgentEnvironmentSnapshot";

import { ModelsResourcesPanel } from "./ModelsResourcesPanel";

vi.mock("../model-resources/ModelResourcePanel", () => ({
  ModelResourcePanel: () => <div data-testid="managed-local-model">Managed model controls</div>,
}));

const snapshot: AgentEnvironmentSnapshot = {
  modelControls: {
    routes: [
      {
        id: { provider: "openai-codex", modelId: "gpt-5.6-sol" },
        name: "GPT 5.6 Sol",
        acceptsImages: true,
      },
      {
        id: { provider: "local", modelId: "qwen" },
        name: "Qwen 3.8 27B",
        acceptsImages: false,
      },
    ],
    selectedRoute: { provider: "openai-codex", modelId: "gpt-5.6-sol" },
    efforts: ["low", "high", "max"],
    selectedEffort: "high",
    appliesAfterCurrentStep: false,
  },
  modelRoutes: [
    {
      id: { provider: "openai-codex", modelId: "gpt-5.6-sol" },
      name: "GPT 5.6 Sol",
      acceptsImages: true,
      thinkingLevels: ["low", "high", "max"],
      enabled: true,
    },
    {
      id: { provider: "local", modelId: "qwen" },
      name: "Qwen 3.8 27B",
      acceptsImages: false,
      thinkingLevels: ["low", "medium", "xhigh"],
      enabled: false,
    },
  ],
  resources: {
    skills: [
      {
        id: "project://skills/check",
        name: "Check",
        enabled: true,
        source: "project",
        origin: "topLevel",
      },
    ],
    extensions: [
      {
        id: "piu://extensions/review",
        name: "Review tools",
        enabled: true,
        source: "piu",
        origin: "topLevel",
      },
    ],
    packages: [
      {
        id: "npm:@piu/review@1.0.0",
        name: "Review package",
        enabled: true,
        source: "piu",
        filtered: true,
        installed: true,
      },
      {
        id: "npm:@piu/missing@1.0.0",
        name: "Missing package",
        enabled: false,
        source: "project",
        filtered: false,
        installed: false,
      },
    ],
  },
  diagnostics: [
    {
      resource: "skill",
      kind: "warning",
      message: "Check could not load its manifest.",
      path: "/private/project/.pi/skills/check/SKILL.md",
      source: "local",
      sourceScope: "project",
    },
    {
      resource: "runtime",
      kind: "error",
      message: "Pi could not inspect one resource.",
      path: "/private/runtime/internal.json",
      source: "private-runtime-source",
      sourceScope: "piu",
    },
    {
      resource: "extension",
      kind: "info",
      message: "Extension loaded normally.",
      path: null,
      source: null,
      sourceScope: null,
    },
  ],
};

test("groups the complete inventory with accessible scoped switches", async () => {
  const onResourceEnabledChange = vi.fn();
  const user = userEvent.setup();
  render(
    <ModelsResourcesPanel
      onResourceEnabledChange={onResourceEnabledChange}
      onRetry={vi.fn()}
      snapshot={snapshot}
    />,
  );

  for (const heading of [
    "Model routes",
    "Skills",
    "Extensions",
    "Packages",
    "Managed local model",
  ]) {
    expect(screen.getByRole("heading", { name: heading })).toBeVisible();
  }
  expect(screen.getByTestId("managed-local-model")).toBeVisible();
  expect(screen.getByText("Required")).toBeVisible();
  expect(screen.getAllByText("Più").length).toBeGreaterThan(0);
  expect(screen.getAllByText("Project").length).toBeGreaterThan(0);

  await user.click(screen.getByRole("switch", { name: "Review tools" }));
  expect(onResourceEnabledChange).toHaveBeenCalledWith(
    { kind: "extension", id: "piu://extensions/review" },
    false,
    "global",
  );
  await user.click(screen.getByRole("switch", { name: "Check" }));
  expect(onResourceEnabledChange).toHaveBeenLastCalledWith(
    { kind: "skill", id: "project://skills/check" },
    false,
    "project",
  );
});

test("shows exceptional load state without exposing runtime paths", () => {
  render(
    <ModelsResourcesPanel
      onResourceEnabledChange={vi.fn()}
      onRetry={vi.fn()}
      snapshot={snapshot}
    />,
  );

  expect(screen.getByText("Project version in use")).toBeVisible();
  expect(screen.getByText("Couldn’t load")).toBeVisible();
  expect(screen.getByText("Check could not load its manifest.")).toBeVisible();
  expect(screen.getByText("Pi could not inspect one resource.")).toBeVisible();
  expect(screen.queryByText("Extension loaded normally.")).not.toBeInTheDocument();
  expect(
    screen.queryByText(/private\/project|private\/runtime|private-runtime-source/),
  ).not.toBeInTheDocument();
});

test("disables only the pending resource and exposes recovery and deferred status", async () => {
  const onRetry = vi.fn();
  const user = userEvent.setup();
  render(
    <ModelsResourcesPanel
      error="Più couldn’t refresh these resources."
      onResourceEnabledChange={vi.fn()}
      onRetry={onRetry}
      pendingResourceId={{ kind: "skill", id: "project://skills/check" }}
      snapshot={snapshot}
      status="Active chats will use this change after the current step."
    />,
  );

  expect(screen.getByRole("alert")).toHaveTextContent("Più couldn’t refresh these resources.");
  expect(screen.getByRole("status")).toHaveTextContent(
    "Active chats will use this change after the current step.",
  );
  expect(screen.getByRole("switch", { name: "Check" })).toHaveAttribute("aria-disabled", "true");
  expect(screen.getByRole("switch", { name: "Review tools" })).not.toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await user.click(screen.getByRole("button", { name: "Retry" }));
  expect(onRetry).toHaveBeenCalledOnce();
});

test("renders a compact loading inventory without blocking managed model controls", () => {
  render(
    <ModelsResourcesPanel
      loading
      onResourceEnabledChange={vi.fn()}
      onRetry={vi.fn()}
      snapshot={null}
    />,
  );

  expect(screen.getByRole("status", { name: "Loading models and resources" })).toBeVisible();
  expect(screen.getByTestId("managed-local-model")).toBeVisible();
});

test("does not claim an inspection is still loading after it fails", () => {
  render(
    <ModelsResourcesPanel
      error="Più couldn’t inspect this project."
      onResourceEnabledChange={vi.fn()}
      onRetry={vi.fn()}
      snapshot={null}
    />,
  );

  expect(screen.getByRole("alert")).toBeVisible();
  expect(
    screen.queryByRole("status", { name: "Loading models and resources" }),
  ).not.toBeInTheDocument();
});

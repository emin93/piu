import { beforeEach, expect, test, vi } from "vitest";

import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";

import { tauriProjectModelControlsAdapter } from "./agent-environment";

const boundary = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: boundary.invoke }));

const controls: ModelControlsSnapshot = {
  appliesAfterCurrentStep: false,
  efforts: ["low", "medium", "xhigh"],
  routes: [
    {
      acceptsImages: true,
      id: { modelId: "qwen3.8-27b", provider: "piu-local" },
      name: "Qwen 3.8 27B",
    },
  ],
  selectedEffort: "medium",
  selectedRoute: { modelId: "qwen3.8-27b", provider: "piu-local" },
};

beforeEach(() => {
  boundary.invoke.mockReset();
  boundary.invoke.mockResolvedValue(controls);
});

test("project model controls use the typed agent environment commands", async () => {
  await expect(tauriProjectModelControlsAdapter.get(7)).resolves.toEqual(controls);
  await expect(
    tauriProjectModelControlsAdapter.selectRoute(7, {
      modelId: "gpt-5.6-sol",
      provider: "openai-codex",
    }),
  ).resolves.toEqual(controls);
  await expect(tauriProjectModelControlsAdapter.selectEffort(7, "xhigh")).resolves.toEqual(
    controls,
  );

  expect(boundary.invoke.mock.calls).toEqual([
    ["get_project_model_controls", { request: { projectId: 7 } }],
    [
      "select_project_model_route",
      {
        request: {
          projectId: 7,
          route: { modelId: "gpt-5.6-sol", provider: "openai-codex" },
        },
      },
    ],
    ["select_project_reasoning_effort", { request: { projectId: 7, effort: "xhigh" } }],
  ]);
});

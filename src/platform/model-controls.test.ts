import { beforeEach, expect, test, vi } from "vitest";

import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";

import { tauriModelControlsAdapter } from "./model-controls";

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

test("model controls use the typed chat runtime commands", async () => {
  await expect(tauriModelControlsAdapter.get("chat-7")).resolves.toEqual(controls);
  await expect(
    tauriModelControlsAdapter.selectRoute("chat-7", {
      modelId: "gpt-5.6-sol",
      provider: "openai-codex",
    }),
  ).resolves.toEqual(controls);
  await expect(tauriModelControlsAdapter.selectEffort("chat-7", "xhigh")).resolves.toEqual(
    controls,
  );

  expect(boundary.invoke.mock.calls).toEqual([
    ["get_model_controls", { request: { chatId: "chat-7" } }],
    [
      "select_model_route",
      {
        request: {
          chatId: "chat-7",
          route: { modelId: "gpt-5.6-sol", provider: "openai-codex" },
        },
      },
    ],
    ["select_reasoning_effort", { request: { chatId: "chat-7", effort: "xhigh" } }],
  ]);
});

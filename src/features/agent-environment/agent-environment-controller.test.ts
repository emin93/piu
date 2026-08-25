import { expect, test, vi } from "vitest";

import type { AgentEnvironmentSnapshot } from "@/generated/AgentEnvironmentSnapshot";
import type { AgentResourcePreferenceChange } from "@/generated/AgentResourcePreferenceChange";
import type { AgentEnvironmentAdapter } from "@/platform/agent-environment";

import { AgentEnvironmentController } from "./agent-environment-controller";

const resource = { kind: "skill", id: "project://skills/check" } as const;

const environment: AgentEnvironmentSnapshot = {
  modelControls: {
    routes: [
      {
        acceptsImages: true,
        id: { provider: "openai-codex", modelId: "gpt-5.6-sol" },
        name: "GPT 5.6 Sol",
      },
    ],
    selectedRoute: { provider: "openai-codex", modelId: "gpt-5.6-sol" },
    efforts: ["high"],
    selectedEffort: "high",
    appliesAfterCurrentStep: false,
  },
  modelRoutes: [
    {
      acceptsImages: true,
      enabled: true,
      id: { provider: "openai-codex", modelId: "gpt-5.6-sol" },
      name: "GPT 5.6 Sol",
      source: "piu",
      thinkingLevels: ["high"],
    },
  ],
  resources: {
    extensions: [],
    packages: [],
    skills: [
      {
        enabled: true,
        id: resource.id,
        name: "Check",
        origin: "topLevel",
        source: "project",
      },
    ],
  },
  diagnostics: [],
};

const refreshedEnvironment: AgentEnvironmentSnapshot = {
  ...environment,
  resources: {
    ...environment.resources,
    skills: [{ ...environment.resources.skills[0], enabled: false }],
  },
};

const change: AgentResourcePreferenceChange = {
  deferredChatCount: 1,
  enabled: false,
  resource,
  restartFailedChatCount: 0,
  scope: "project",
  status: "deferred",
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

test("serializes resource changes and preserves the first change status", async () => {
  const pendingChange = deferred<AgentResourcePreferenceChange>();
  const adapter: AgentEnvironmentAdapter = {
    get: vi.fn().mockResolvedValueOnce(environment).mockResolvedValueOnce(refreshedEnvironment),
    setEnabled: vi.fn(() => pendingChange.promise),
  };
  const controller = new AgentEnvironmentController(17, adapter);
  await controller.load();

  const first = controller.setResourceEnabled(resource, false, "project");
  await controller.setResourceEnabled(
    { kind: "extension", id: "piu://extensions/review" },
    false,
    "global",
  );

  expect(adapter.setEnabled).toHaveBeenCalledOnce();
  expect(controller.getSnapshot()).toMatchObject({
    pendingResource: resource,
    phase: "changing",
  });

  pendingChange.resolve(change);
  await first;

  expect(adapter.get).toHaveBeenCalledTimes(2);
  expect(controller.getSnapshot()).toMatchObject({
    environment: refreshedEnvironment,
    error: null,
    pendingResource: null,
    phase: "ready",
    status: "1 active chat will use this change after the current step.",
  });
});

test("a committed change with a failed refresh retries only the snapshot load", async () => {
  const adapter: AgentEnvironmentAdapter = {
    get: vi
      .fn()
      .mockResolvedValueOnce(environment)
      .mockRejectedValueOnce(new Error("fixture refresh failure"))
      .mockResolvedValueOnce(refreshedEnvironment),
    setEnabled: vi.fn().mockResolvedValue({
      ...change,
      deferredChatCount: 0,
      restartFailedChatCount: 1,
      status: "restartFailed",
    }),
  };
  const controller = new AgentEnvironmentController(17, adapter);
  await controller.load();

  await controller.setResourceEnabled(resource, false, "project");

  expect(controller.getSnapshot()).toEqual({
    environment: null,
    error:
      "The change was saved, but Più couldn’t refresh models and resources. Retry to load the saved state.",
    pendingResource: null,
    phase: "failed",
    status: "The change was saved, but Più couldn’t restart 1 idle chat. Open it to reconnect.",
  });

  await controller.retry();

  expect(adapter.setEnabled).toHaveBeenCalledOnce();
  expect(adapter.get).toHaveBeenCalledTimes(3);
  expect(controller.getSnapshot()).toMatchObject({
    environment: refreshedEnvironment,
    error: null,
    phase: "ready",
  });
});

import { expect, test, vi } from "vitest";

import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";
import type { ModelControlsAdapter } from "@/platform/model-controls";

import { ModelControlsController } from "./model-controls-controller";

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  let reject: (reason?: unknown) => void = () => undefined;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

const qwenRoute = { modelId: "qwen3.8-27b", provider: "piu-local" };
const codexRoute = { modelId: "gpt-5.6-sol", provider: "openai-codex" };
const initialControls: ModelControlsSnapshot = {
  appliesAfterCurrentStep: false,
  efforts: ["low", "medium", "xhigh"],
  routes: [
    { acceptsImages: true, id: qwenRoute, name: "Qwen 3.8 27B" },
    { acceptsImages: true, id: codexRoute, name: "GPT-5.6 Sol" },
  ],
  selectedEffort: "medium",
  selectedRoute: qwenRoute,
};

function adapter(overrides: Partial<ModelControlsAdapter> = {}): ModelControlsAdapter {
  return {
    get: vi.fn().mockResolvedValue(initialControls),
    selectEffort: vi.fn().mockResolvedValue(initialControls),
    selectRoute: vi.fn().mockResolvedValue(initialControls),
    ...overrides,
  };
}

test("loading publishes Pi's effective model controls as one stable snapshot", async () => {
  const controls = new ModelControlsController("chat-7", adapter());
  const listener = vi.fn();
  controls.subscribe(listener);

  expect(controls.getSnapshot()).toMatchObject({ controls: null, error: null, phase: "loading" });
  await controls.load();

  const ready = controls.getSnapshot();
  expect(ready).toEqual({ controls: initialControls, error: null, pending: null, phase: "ready" });
  expect(controls.getSnapshot()).toBe(ready);
  expect(listener).toHaveBeenCalledOnce();
});

test("the same controller loads model controls for a project target", async () => {
  const projectAdapter: ModelControlsAdapter<number> = {
    get: vi.fn().mockResolvedValue(initialControls),
    selectEffort: vi.fn().mockResolvedValue(initialControls),
    selectRoute: vi.fn().mockResolvedValue(initialControls),
  };
  const controls = new ModelControlsController(7, projectAdapter);

  await controls.load();
  await controls.selectEffort("xhigh");

  expect(projectAdapter.get).toHaveBeenCalledWith(7);
  expect(projectAdapter.selectEffort).toHaveBeenCalledWith(7, "xhigh");
});

test("a route selection is visible while pending and adopts only Pi's returned efforts", async () => {
  const pending = deferred<ModelControlsSnapshot>();
  const modelAdapter = adapter({ selectRoute: vi.fn().mockReturnValue(pending.promise) });
  const controls = new ModelControlsController("chat-7", modelAdapter);
  await controls.load();

  const selection = controls.selectRoute(codexRoute);
  expect(controls.getSnapshot()).toMatchObject({
    controls: initialControls,
    error: null,
    pending: { kind: "route", route: codexRoute },
    phase: "changing",
  });

  const changed: ModelControlsSnapshot = {
    ...initialControls,
    appliesAfterCurrentStep: true,
    efforts: ["high", "max"],
    selectedEffort: "max",
    selectedRoute: codexRoute,
  };
  pending.resolve(changed);
  await selection;

  expect(controls.getSnapshot()).toEqual({
    controls: changed,
    error: null,
    pending: null,
    phase: "ready",
  });
  expect(modelAdapter.selectRoute).toHaveBeenCalledWith("chat-7", codexRoute);
});

test("a rejected change retains the working route and can be retried", async () => {
  const selectRoute = vi
    .fn<ModelControlsAdapter["selectRoute"]>()
    .mockRejectedValueOnce(new Error("route unavailable"))
    .mockResolvedValueOnce({
      ...initialControls,
      selectedRoute: codexRoute,
    });
  const controls = new ModelControlsController("chat-7", adapter({ selectRoute }));
  await controls.load();

  await controls.selectRoute(codexRoute);
  expect(controls.getSnapshot()).toEqual({
    controls: initialControls,
    error: "Couldn’t switch to GPT-5.6 Sol. Still using Qwen 3.8 27B.",
    pending: null,
    phase: "failed",
  });

  await controls.retry();
  expect(selectRoute).toHaveBeenCalledTimes(2);
  expect(controls.getSnapshot()).toMatchObject({
    controls: { selectedRoute: codexRoute },
    error: null,
    phase: "ready",
  });
});

test.each([
  {
    change: (controls: ModelControlsController) => controls.selectRoute(codexRoute),
    override: {
      selectRoute: vi.fn().mockRejectedValue({
        code: "inferenceRollbackFailed",
        message: "Pi couldn’t safely restore the previous model. Reopen the chat and try again.",
      }),
    },
  },
  {
    change: (controls: ModelControlsController) => controls.selectEffort("xhigh"),
    override: {
      selectEffort: vi.fn().mockRejectedValue({
        code: "inferenceRollbackFailed",
        message: "Pi couldn’t safely restore the previous model. Reopen the chat and try again.",
      }),
    },
  },
])(
  "a failed inference rollback reports the recovery action without offering retry",
  async (testCase) => {
    const controls = new ModelControlsController<string>("chat-7", adapter(testCase.override));
    await controls.load();

    await testCase.change(controls);

    expect(controls.getSnapshot()).toEqual({
      controls: initialControls,
      error: "Pi couldn’t safely restore the previous model. Reopen the chat and try again.",
      pending: null,
      phase: "failed",
    });
    await controls.retry();
    expect(testCase.override.selectRoute ?? testCase.override.selectEffort).toHaveBeenCalledOnce();
  },
);

test("reasoning changes use only a level present in the effective snapshot", async () => {
  const selectEffort = vi.fn<ModelControlsAdapter["selectEffort"]>();
  const controls = new ModelControlsController("chat-7", adapter({ selectEffort }));
  await controls.load();

  await controls.selectEffort("max");

  expect(selectEffort).not.toHaveBeenCalled();
  expect(controls.getSnapshot()).toMatchObject({
    controls: initialControls,
    error: "Maximum is not available for Qwen 3.8 27B.",
    phase: "failed",
  });
});

test("the completed safe step clears only the transient application status", async () => {
  const controls = new ModelControlsController(
    "chat-7",
    adapter({
      get: vi.fn().mockResolvedValue({ ...initialControls, appliesAfterCurrentStep: true }),
    }),
  );
  await controls.load();

  controls.markCurrentStepApplied();

  expect(controls.getSnapshot()).toMatchObject({
    controls: { appliesAfterCurrentStep: false },
    error: null,
    phase: "ready",
  });
});

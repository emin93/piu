import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_ENVIRONMENT_SNAPSHOT_BYTES,
  runEnvironmentLauncher,
} from "./environment-launcher-runtime.mjs";

function dependencies({ inspectEnvironment, write }) {
  class InMemoryModelsStore {}

  return {
    ai: {
      getSupportedThinkingLevels: () => ["off", "high"],
      InMemoryModelsStore,
    },
    inspectEnvironment,
    locking: { default: { lock: async () => undefined } },
    pi: { ModelRuntime: {} },
    write,
  };
}

const arguments_ = [
  "--cwd",
  "/private/tmp/project",
  "--agent-dir",
  "/private/tmp/app/agent",
  "--credential-lock-dir",
  "/private/tmp/app/locks",
  "--resource-preferences",
  '{"global":[],"project":[]}',
];

test("wires the public SDK stores and emits exactly one bounded JSON snapshot", async () => {
  const records = [];
  let inspected;
  await runEnvironmentLauncher(
    arguments_,
    dependencies({
      inspectEnvironment: async (config, sdk) => {
        inspected = { config, sdk };
        return { modelRoutes: [], resources: {}, diagnostics: [] };
      },
      write: (record) => records.push(record),
    }),
  );

  assert.deepEqual(inspected.config, {
    cwd: "/private/tmp/project",
    agentDirectory: "/private/tmp/app/agent",
    credentialLockDirectory: "/private/tmp/app/locks",
    resourcePreferences: { global: [], project: [] },
  });
  assert.equal(inspected.sdk.modelsStore.constructor.name, "InMemoryModelsStore");
  assert.deepEqual(inspected.sdk.getSupportedThinkingLevels(), ["off", "high"]);
  assert.equal(typeof inspected.sdk.credentials.read, "function");
  assert.equal(inspected.sdk.pi.ModelRuntime instanceof Object, true);
  assert.deepEqual(records, ['{"modelRoutes":[],"resources":{},"diagnostics":[]}\n']);
});

test("rejects an oversized result before writing any stdout", async () => {
  let writes = 0;
  await assert.rejects(
    runEnvironmentLauncher(
      arguments_,
      dependencies({
        inspectEnvironment: async () => ({
          diagnostics: ["x".repeat(MAX_ENVIRONMENT_SNAPSHOT_BYTES)],
        }),
        write: () => {
          writes += 1;
        },
      }),
    ),
    /exceeds the output limit/,
  );
  assert.equal(writes, 0);
});

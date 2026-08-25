import assert from "node:assert/strict";
import { homedir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { inspectPiuEnvironment } from "./environment-inspection.mjs";

const paths = {
  agentDirectory: "/Users/test/Library/Application Support/ch.emin.piu/agent",
  cwd: "/private/tmp/piu/worktrees/chat-1",
};

function createPiContract() {
  const resolved = {
    extensions: [
      {
        path: `${paths.cwd}/.pi/extensions/route.mjs`,
        enabled: true,
        metadata: { source: "local", scope: "project", origin: "top-level" },
      },
    ],
    skills: [
      {
        path: `${paths.agentDirectory}/skills/review/SKILL.md`,
        enabled: true,
        metadata: { source: "local", scope: "user", origin: "top-level" },
      },
      {
        path: `${paths.cwd}/.pi/skills/disabled/SKILL.md`,
        enabled: false,
        metadata: { source: "local", scope: "project", origin: "top-level" },
      },
    ],
    prompts: [],
    themes: [],
  };
  const settingsManager = {
    applyOverrides(overrides) {
      settingsOverrides = overrides;
    },
    drainErrors() {
      return [
        {
          scope: "project",
          path: `${paths.cwd}/.pi/settings.json`,
          error: new Error("fixture settings warning"),
        },
      ];
    },
  };
  const isolatedSettingsManager = { kind: "isolated" };
  const modelRuntime = {
    async getAvailable() {
      return [
        { provider: "piu-contract", id: "reasoning", name: "Reasoning", reasoning: true },
        { provider: "piu-contract", id: "plain", name: "Plain", reasoning: false },
      ];
    },
    getError() {
      return "fixture model warning";
    },
  };
  let resourceLoaderOptions;
  let settingsOverrides;
  let modelRuntimeOptions;

  class DefaultPackageManager {
    async resolve(onMissing) {
      assert.equal(await onMissing("npm:@piu/missing@1.0.0"), "skip");
      return resolved;
    }

    listConfiguredPackages() {
      return [
        {
          source: "npm:@piu/missing@1.0.0",
          scope: "user",
          filtered: false,
        },
      ];
    }
  }

  const pi = {
    DefaultPackageManager,
    ModelRuntime: {
      async create(options) {
        modelRuntimeOptions = options;
        return modelRuntime;
      },
    },
    SettingsManager: {
      create() {
        return settingsManager;
      },
      inMemory() {
        return isolatedSettingsManager;
      },
    },
    async createAgentSessionServices(options) {
      resourceLoaderOptions = options.resourceLoaderOptions;
      assert.equal(options.settingsManager, isolatedSettingsManager);
      return {
        diagnostics: [{ type: "warning", message: "fixture runtime warning" }],
        modelRuntime,
        resourceLoader: {
          getExtensions() {
            return {
              errors: [
                {
                  path: resolved.extensions[0].path,
                  error: "fixture extension warning",
                },
              ],
            };
          },
          getSkills() {
            return {
              diagnostics: [
                {
                  type: "collision",
                  message: 'name "review" collision',
                  path: resolved.skills[0].path,
                },
              ],
            };
          },
        },
      };
    },
  };

  return {
    get resourceLoaderOptions() {
      return resourceLoaderOptions;
    },
    get settingsOverrides() {
      return settingsOverrides;
    },
    get modelRuntimeOptions() {
      return modelRuntimeOptions;
    },
    pi,
  };
}

test("discovers effective routes and inventories isolated Più resources", async () => {
  const contract = createPiContract();
  const credentials = { read: async () => undefined };
  const modelsStore = {};

  const result = await inspectPiuEnvironment(paths, {
    canonicalizePath: async (path) => path,
    credentials,
    getSupportedThinkingLevels(model) {
      return model.reasoning ? ["off", "low", "max"] : ["off"];
    },
    modelsStore,
    pi: contract.pi,
  });

  assert.deepEqual(result.modelRoutes, [
    {
      provider: "piu-contract",
      id: "reasoning",
      name: "Reasoning",
      thinkingLevels: ["off", "low", "max"],
    },
    {
      provider: "piu-contract",
      id: "plain",
      name: "Plain",
      thinkingLevels: ["off"],
    },
  ]);
  assert.deepEqual(contract.modelRuntimeOptions, {
    allowModelNetwork: false,
    credentials,
    modelsPath: `${paths.agentDirectory}/models.json`,
    modelsStore,
  });
  assert.deepEqual(result.resources, {
    extensions: [
      {
        path: `${paths.cwd}/.pi/extensions/route.mjs`,
        enabled: true,
        source: "local",
        scope: "project",
        origin: "top-level",
      },
    ],
    skills: [
      {
        path: `${paths.agentDirectory}/skills/review/SKILL.md`,
        enabled: true,
        source: "local",
        scope: "user",
        origin: "top-level",
      },
      {
        path: `${paths.cwd}/.pi/skills/disabled/SKILL.md`,
        enabled: false,
        source: "local",
        scope: "project",
        origin: "top-level",
      },
    ],
    packages: [
      {
        source: "npm:@piu/missing@1.0.0",
        scope: "user",
        filtered: false,
        installedPath: undefined,
      },
    ],
  });
  assert.deepEqual(contract.resourceLoaderOptions, {
    additionalExtensionPaths: [`${paths.cwd}/.pi/extensions/route.mjs`],
    additionalSkillPaths: [`${paths.agentDirectory}/skills/review/SKILL.md`],
    noContextFiles: true,
    noExtensions: true,
    noPromptTemplates: true,
    noSkills: true,
    noThemes: true,
  });
  assert.equal(contract.settingsOverrides.npmCommand[0], process.execPath);
  assert.equal(contract.settingsOverrides.npmCommand.includes("--eval"), false);
  assert.match(
    contract.settingsOverrides.npmCommand.join(" "),
    /Application Support\/ch\.emin\.piu\/agent\/\.piu-empty-global-npm/,
  );
  assert.deepEqual(result.diagnostics, [
    {
      resourceType: "package",
      type: "warning",
      message: "Configured package is unavailable; automatic installation is disabled",
      source: "npm:@piu/missing@1.0.0",
    },
    {
      resourceType: "settings",
      type: "error",
      message: "fixture settings warning",
      path: `${paths.cwd}/.pi/settings.json`,
      scope: "project",
    },
    {
      resourceType: "runtime",
      type: "warning",
      message: "fixture runtime warning",
    },
    {
      resourceType: "extension",
      type: "error",
      message: "fixture extension warning",
      path: `${paths.cwd}/.pi/extensions/route.mjs`,
    },
    {
      resourceType: "skill",
      type: "collision",
      message: 'name "review" collision',
      path: `${paths.agentDirectory}/skills/review/SKILL.md`,
    },
    {
      resourceType: "model",
      type: "warning",
      message: "fixture model warning",
    },
  ]);
});

test("rejects paths that could fall back to standalone Pi state", async () => {
  const { pi } = createPiContract();

  await assert.rejects(
    inspectPiuEnvironment(
      { ...paths, agentDirectory: ".pi/agent" },
      { credentials: {}, getSupportedThinkingLevels: () => ["off"], pi },
    ),
    /agent directory must be an absolute path/,
  );
  await assert.rejects(
    inspectPiuEnvironment(
      { ...paths, agentDirectory: join(homedir(), ".pi", "agent") },
      { credentials: {}, getSupportedThinkingLevels: () => ["off"], modelsStore: {}, pi },
    ),
    /standalone Pi agent directory is not allowed/,
  );
});

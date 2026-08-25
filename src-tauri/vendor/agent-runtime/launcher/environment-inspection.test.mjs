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
  let availableModels = [
    { provider: "piu-contract", id: "reasoning", name: "Reasoning", reasoning: true },
    { provider: "piu-contract", id: "plain", name: "Plain", reasoning: false },
  ];
  const modelRuntime = {
    async getAvailable() {
      return availableModels;
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
      options.resourceLoaderOptions.extensionsOverride({
        extensions: [],
        errors: [],
        runtime: {
          pendingProviderRegistrations: [
            {
              name: "piu-contract",
              config: {},
              extensionPath: resolved.extensions[0].path,
            },
            {
              name: "project-contract",
              config: {},
              extensionPath: resolved.extensions[0].path,
            },
          ],
          pendingNativeProviderRegistrations: [],
        },
      });
      availableModels = [
        { ...availableModels[0], name: "Project reasoning override" },
        availableModels[1],
        { provider: "project-contract", id: "project-model", name: "Project", reasoning: true },
      ];
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
      name: "Project reasoning override",
      acceptsImages: false,
      thinkingLevels: ["off", "low", "max"],
      scope: "project",
      owner: { extensionId: `${paths.cwd}/.pi/extensions/route.mjs` },
    },
    {
      provider: "piu-contract",
      id: "plain",
      name: "Plain",
      acceptsImages: false,
      thinkingLevels: ["off"],
      scope: "project",
      owner: { extensionId: `${paths.cwd}/.pi/extensions/route.mjs` },
    },
    {
      provider: "project-contract",
      id: "project-model",
      name: "Project",
      acceptsImages: false,
      thinkingLevels: ["off", "low", "max"],
      scope: "project",
      owner: { extensionId: `${paths.cwd}/.pi/extensions/route.mjs` },
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
        id: `${paths.cwd}/.pi/extensions/route.mjs`,
        name: "route",
        path: `${paths.cwd}/.pi/extensions/route.mjs`,
        enabled: true,
        source: "local",
        scope: "project",
        origin: "top-level",
      },
    ],
    skills: [
      {
        id: `${paths.agentDirectory}/skills/review/SKILL.md`,
        name: "review",
        path: `${paths.agentDirectory}/skills/review/SKILL.md`,
        enabled: true,
        source: "local",
        scope: "user",
        origin: "top-level",
      },
      {
        id: `${paths.cwd}/.pi/skills/disabled/SKILL.md`,
        name: "disabled",
        path: `${paths.cwd}/.pi/skills/disabled/SKILL.md`,
        enabled: false,
        source: "local",
        scope: "project",
        origin: "top-level",
      },
    ],
    packages: [
      {
        id: "npm:@piu/missing@1.0.0",
        name: "npm:@piu/missing@1.0.0",
        source: "npm:@piu/missing@1.0.0",
        scope: "user",
        filtered: false,
        installedPath: undefined,
      },
    ],
  });
  const { extensionsOverride, ...resourceLoaderOptions } = contract.resourceLoaderOptions;
  assert.equal(typeof extensionsOverride, "function");
  assert.deepEqual(resourceLoaderOptions, {
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

async function inspectModelProvenance({
  extensionScope,
  extensionSource = "local",
  extensionOrigin = "top-level",
  initialModels,
  registrations,
  effectiveModels,
  resourcePreferences = { global: [], project: [] },
}) {
  const extensionPath =
    extensionScope === "user"
      ? `${paths.agentDirectory}/extensions/route.mjs`
      : `${paths.cwd}/.pi/extensions/route.mjs`;
  let availableModels = initialModels;
  let loadedExtensionPaths = [];
  const modelRuntime = {
    async getAvailable() {
      return availableModels;
    },
    getError() {
      return undefined;
    },
  };
  const settingsManager = {
    applyOverrides() {},
    drainErrors() {
      return [];
    },
  };

  class DefaultPackageManager {
    async resolve() {
      return {
        extensions: [
          {
            path: extensionPath,
            enabled: true,
            metadata: {
              source: extensionSource,
              scope: extensionScope,
              origin: extensionOrigin,
            },
          },
        ],
        skills: [],
        prompts: [],
        themes: [],
      };
    }

    listConfiguredPackages() {
      return extensionOrigin === "package"
        ? [
            {
              source: extensionSource,
              scope: extensionScope,
              filtered: false,
              installedPath: extensionPath,
            },
          ]
        : [];
    }
  }

  const pi = {
    DefaultPackageManager,
    ModelRuntime: {
      async create() {
        return modelRuntime;
      },
    },
    SettingsManager: {
      create() {
        return settingsManager;
      },
      inMemory() {
        return {};
      },
    },
    async createAgentSessionServices(options) {
      loadedExtensionPaths = options.resourceLoaderOptions.additionalExtensionPaths;
      const extensionLoaded =
        options.resourceLoaderOptions.additionalExtensionPaths.includes(extensionPath);
      options.resourceLoaderOptions.extensionsOverride({
        extensions: [],
        errors: [],
        runtime: {
          pendingProviderRegistrations: extensionLoaded
            ? registrations.map((registration) => ({
                ...registration,
                extensionPath,
              }))
            : [],
          pendingNativeProviderRegistrations: [],
        },
      });
      availableModels = extensionLoaded ? effectiveModels : initialModels;
      return {
        diagnostics: [],
        modelRuntime,
        resourceLoader: {
          getExtensions() {
            return { errors: [] };
          },
          getSkills() {
            return { skills: [], diagnostics: [] };
          },
        },
      };
    },
  };

  const result = await inspectPiuEnvironment(
    { ...paths, resourcePreferences },
    {
      canonicalizePath: async (path) => path,
      credentials: {},
      getSupportedThinkingLevels: () => ["off"],
      modelsStore: {},
      pi,
    },
  );
  return {
    extensionPath,
    loadedExtensionPaths,
    result,
  };
}

const userModel = {
  provider: "shared-contract",
  id: "model",
  name: "Shared model",
  reasoning: false,
};

for (const scenario of [
  {
    name: "global extension model additions as user routes",
    extensionScope: "user",
    initialModels: [userModel],
    registrations: [{ name: "global-contract", config: {} }],
    effectiveModels: [
      userModel,
      { provider: "global-contract", id: "model", name: "Global model", reasoning: false },
    ],
    route: { provider: "global-contract", id: "model" },
    scope: "user",
  },
  {
    name: "project extension model additions as project routes",
    extensionScope: "project",
    initialModels: [userModel],
    registrations: [{ name: "project-contract", config: {} }],
    effectiveModels: [
      userModel,
      { provider: "project-contract", id: "model", name: "Project model", reasoning: false },
    ],
    route: { provider: "project-contract", id: "model" },
    scope: "project",
  },
  {
    name: "project extension provider overrides as project routes",
    extensionScope: "project",
    initialModels: [userModel],
    registrations: [{ name: "shared-contract", config: { baseUrl: "https://project.invalid" } }],
    effectiveModels: [userModel],
    route: { provider: "shared-contract", id: "model" },
    scope: "project",
  },
  {
    name: "package extension model additions with package ownership",
    extensionScope: "user",
    extensionSource: "npm:@piu/models@1.0.0",
    extensionOrigin: "package",
    initialModels: [userModel],
    registrations: [{ name: "package-contract", config: {} }],
    effectiveModels: [
      userModel,
      { provider: "package-contract", id: "model", name: "Package model", reasoning: false },
    ],
    route: { provider: "package-contract", id: "model" },
    scope: "user",
  },
]) {
  test(`classifies ${scenario.name}`, async () => {
    const inspected = await inspectModelProvenance(scenario);
    const route = inspected.result.modelRoutes.find(
      ({ provider, id }) => provider === scenario.route.provider && id === scenario.route.id,
    );

    assert.equal(route?.scope, scenario.scope);
    assert.deepEqual(route?.owner, {
      extensionId:
        scenario.extensionScope === "user"
          ? `${paths.agentDirectory}/extensions/route.mjs`
          : `${paths.cwd}/.pi/extensions/route.mjs`,
      ...(scenario.extensionOrigin === "package" ? { packageId: scenario.extensionSource } : {}),
    });
  });
}

test("applies Più extension and package preferences before provider code executes", async () => {
  const scenario = {
    extensionScope: "user",
    extensionSource: "npm:@piu/models@1.0.0",
    extensionOrigin: "package",
    initialModels: [userModel],
    registrations: [{ name: "package-contract", config: {} }],
    effectiveModels: [
      userModel,
      { provider: "package-contract", id: "model", name: "Package model", reasoning: false },
    ],
  };
  const disabled = await inspectModelProvenance({
    ...scenario,
    resourcePreferences: {
      global: [{ kind: "package", id: "npm:@piu/models@1.0.0", enabled: false }],
      project: [],
    },
  });
  assert.deepEqual(disabled.loadedExtensionPaths, []);
  assert.equal(
    disabled.result.modelRoutes.some(({ provider }) => provider === "package-contract"),
    false,
  );
  assert.equal(disabled.result.resources.extensions.length, 1);
  assert.equal(disabled.result.resources.packages.length, 1);

  const reenabled = await inspectModelProvenance({
    ...scenario,
    resourcePreferences: {
      global: [{ kind: "package", id: "npm:@piu/models@1.0.0", enabled: false }],
      project: [{ kind: "extension", id: disabled.extensionPath, enabled: true }],
    },
  });
  assert.deepEqual(reenabled.loadedExtensionPaths, [reenabled.extensionPath]);
  assert.equal(
    reenabled.result.modelRoutes.some(({ provider }) => provider === "package-contract"),
    true,
  );
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

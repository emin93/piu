import assert from "node:assert/strict";
import { access, mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";

import { inspectPiuEnvironment } from "../src-tauri/vendor/agent-runtime/launcher/environment-inspection.mjs";

const runtimePackageRoot = resolve("src-tauri/vendor/agent-runtime/runtime/pi");
const pi = await import(
  pathToFileURL(
    join(runtimePackageRoot, "node_modules/@earendil-works/pi-coding-agent/dist/index.js"),
  )
);
const { getSupportedThinkingLevels, InMemoryCredentialStore, InMemoryModelsStore } = await import(
  pathToFileURL(join(runtimePackageRoot, "node_modules/@earendil-works/pi-ai/dist/index.js"))
);

async function tree(root) {
  return (await readdir(root, { recursive: true })).map(String).sort();
}

test("the pinned Pi SDK inspects only explicitly isolated Più resources without installing", async () => {
  const fixtureRoot = await mkdtemp(join(tmpdir(), "piu-environment-contract-"));
  const paths = {
    agentDirectory: join(fixtureRoot, "app", "agent"),
    cwd: join(fixtureRoot, "worktree"),
    home: join(fixtureRoot, "standalone-home"),
  };
  const globalSkill = join(paths.agentDirectory, "skills", "shared-review");
  const globalExtensions = join(paths.agentDirectory, "extensions");
  const projectSkill = join(paths.cwd, ".pi", "skills", "shared-review");
  const projectExtensions = join(paths.cwd, ".pi", "extensions");
  const providerExecutionMarker = join(fixtureRoot, "provider-executed");
  const standaloneAgent = join(paths.home, ".pi", "agent");
  const standaloneSkill = join(paths.home, ".agents", "skills", "standalone-only");
  const previousHome = process.env.HOME;

  try {
    await Promise.all([
      mkdir(globalSkill, { recursive: true }),
      mkdir(globalExtensions, { recursive: true }),
      mkdir(projectSkill, { recursive: true }),
      mkdir(projectExtensions, { recursive: true }),
      mkdir(join(standaloneAgent, "extensions"), { recursive: true }),
      mkdir(standaloneSkill, { recursive: true }),
    ]);
    await Promise.all([
      writeFile(
        join(paths.agentDirectory, "settings.json"),
        JSON.stringify({ packages: ["npm:@piu-contract/missing-environment-package@0.0.0"] }),
      ),
      writeFile(
        join(globalSkill, "SKILL.md"),
        "---\nname: shared-review\ndescription: Global fixture\n---\nGlobal.\n",
      ),
      writeFile(
        join(globalExtensions, "routes.js"),
        `import { fauxProvider } from "@earendil-works/pi-ai";

export default function (extension) {
  extension.registerProvider(fauxProvider({
    provider: "piu-global-contract",
    models: [{ id: "global", name: "Global", reasoning: false }],
  }).provider);
}
`,
      ),
      writeFile(
        join(projectSkill, "SKILL.md"),
        "---\nname: shared-review\ndescription: Project fixture\n---\nProject.\n",
      ),
      writeFile(
        join(projectExtensions, "route.js"),
        `import { fauxProvider } from "@earendil-works/pi-ai";

export default function (extension) {
  const fixture = fauxProvider({
    provider: "piu-environment-contract",
    models: [
      { id: "deep", name: "Deep", reasoning: true },
      { id: "plain", name: "Plain", reasoning: false },
    ],
  });
  fixture.models[0].thinkingLevelMap = { xhigh: "xhigh", max: "max" };
  extension.registerProvider(fixture.provider);
  extension.registerProvider(fauxProvider({
    provider: "piu-override-contract",
    models: [{ id: "shared", name: "Project base", reasoning: false }],
  }).provider);
  extension.registerProvider(fauxProvider({
    provider: "piu-override-contract",
    models: [{ id: "shared", name: "Project override", reasoning: false }],
  }).provider);
}
`,
      ),
      writeFile(join(projectExtensions, "broken.js"), "export default function ( {\n"),
      writeFile(
        join(projectExtensions, "toggle.js"),
        `import { writeFileSync } from "node:fs";
import { fauxProvider } from "@earendil-works/pi-ai";

export default function (extension) {
  writeFileSync(${JSON.stringify(providerExecutionMarker)}, "executed");
  extension.registerProvider(fauxProvider({
    provider: "piu-toggle-contract",
    models: [{ id: "toggle", name: "Toggle", reasoning: false }],
  }).provider);
}
`,
      ),
      writeFile(join(standaloneAgent, "settings.json"), "{ this is deliberately invalid json"),
      writeFile(
        join(standaloneAgent, "extensions", "standalone.js"),
        `export default function (extension) {
  extension.registerProvider("standalone-only", {
    baseUrl: "https://standalone.invalid",
    apiKey: "standalone",
    api: "openai-responses",
    models: [],
  });
}
`,
      ),
      writeFile(
        join(standaloneSkill, "SKILL.md"),
        "---\nname: standalone-only\ndescription: Must stay isolated\n---\nNever load.\n",
      ),
    ]);
    process.env.HOME = paths.home;
    const appTreeBefore = await tree(join(fixtureRoot, "app"));
    const standaloneTreeBefore = await tree(paths.home);

    const result = await inspectPiuEnvironment(
      { agentDirectory: paths.agentDirectory, cwd: paths.cwd },
      {
        credentials: new InMemoryCredentialStore(),
        getSupportedThinkingLevels,
        modelsStore: new InMemoryModelsStore(),
        pi,
      },
    );

    assert.deepEqual(
      result.modelRoutes.filter(({ provider }) => provider === "piu-environment-contract"),
      [
        {
          provider: "piu-environment-contract",
          id: "deep",
          name: "Deep",
          acceptsImages: true,
          thinkingLevels: ["off", "minimal", "low", "medium", "high", "xhigh", "max"],
          scope: "project",
          owner: { extensionId: join(projectExtensions, "route.js") },
        },
        {
          provider: "piu-environment-contract",
          id: "plain",
          name: "Plain",
          acceptsImages: true,
          thinkingLevels: ["off"],
          scope: "project",
          owner: { extensionId: join(projectExtensions, "route.js") },
        },
      ],
    );
    assert.deepEqual(
      result.modelRoutes.filter(({ provider }) => provider === "piu-global-contract"),
      [
        {
          provider: "piu-global-contract",
          id: "global",
          name: "Global",
          acceptsImages: true,
          thinkingLevels: ["off"],
          scope: "user",
          owner: { extensionId: join(globalExtensions, "routes.js") },
        },
      ],
    );
    assert.deepEqual(
      result.modelRoutes.filter(({ provider }) => provider === "piu-override-contract"),
      [
        {
          provider: "piu-override-contract",
          id: "shared",
          name: "Project override",
          acceptsImages: true,
          thinkingLevels: ["off"],
          scope: "project",
          owner: { extensionId: join(projectExtensions, "route.js") },
        },
      ],
    );
    assert.equal(
      result.resources.skills.some(({ path }) => path.includes("standalone-only")),
      false,
    );
    assert.equal(
      result.resources.extensions.some(({ path }) => path.includes("standalone.js")),
      false,
    );
    assert.equal(
      result.modelRoutes.some(({ provider }) => provider === "standalone-only"),
      false,
    );
    assert.equal(
      result.diagnostics.some(
        ({ resourceType, source, type }) =>
          resourceType === "package" &&
          source === "npm:@piu-contract/missing-environment-package@0.0.0" &&
          type === "warning",
      ),
      true,
    );
    assert.equal(
      result.diagnostics.some(
        ({ resourceType, message, type }) =>
          resourceType === "extension" && type === "error" && message.length > 0,
      ),
      true,
    );
    assert.equal(
      result.diagnostics.some(
        ({ resourceType, type }) => resourceType === "skill" && type === "collision",
      ),
      true,
    );
    await rm(providerExecutionMarker, { force: true });
    const toggleExtension = join(projectExtensions, "toggle.js");
    const disabled = await inspectPiuEnvironment(
      {
        agentDirectory: paths.agentDirectory,
        cwd: paths.cwd,
        resourcePreferences: {
          global: [{ kind: "extension", id: toggleExtension, enabled: false }],
          project: [],
        },
      },
      {
        credentials: new InMemoryCredentialStore(),
        getSupportedThinkingLevels,
        modelsStore: new InMemoryModelsStore(),
        pi,
      },
    );
    await assert.rejects(access(providerExecutionMarker));
    assert.equal(
      disabled.resources.extensions.some(({ id }) => id === toggleExtension),
      true,
    );
    assert.equal(
      disabled.modelRoutes.some(({ provider }) => provider === "piu-toggle-contract"),
      false,
    );

    const reenabled = await inspectPiuEnvironment(
      {
        agentDirectory: paths.agentDirectory,
        cwd: paths.cwd,
        resourcePreferences: {
          global: [{ kind: "extension", id: toggleExtension, enabled: false }],
          project: [{ kind: "extension", id: toggleExtension, enabled: true }],
        },
      },
      {
        credentials: new InMemoryCredentialStore(),
        getSupportedThinkingLevels,
        modelsStore: new InMemoryModelsStore(),
        pi,
      },
    );
    await access(providerExecutionMarker);
    assert.equal(
      reenabled.modelRoutes.some(({ provider }) => provider === "piu-toggle-contract"),
      true,
    );
    await rm(providerExecutionMarker, { force: true });
    assert.deepEqual(await tree(join(fixtureRoot, "app")), appTreeBefore);
    assert.deepEqual(await tree(paths.home), standaloneTreeBefore);
    assert.equal(process.env.HOME, paths.home);
  } finally {
    if (previousHome === undefined) delete process.env.HOME;
    else process.env.HOME = previousHome;
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

test("inspection leaves a missing app agent directory absent on first launch", async () => {
  const fixtureRoot = await mkdtemp(join(tmpdir(), "piu-environment-first-launch-"));
  const agentDirectory = join(fixtureRoot, "app", "agent");
  const cwd = join(fixtureRoot, "worktree");
  const home = join(fixtureRoot, "standalone-home");
  const previousHome = process.env.HOME;

  try {
    await Promise.all([mkdir(cwd, { recursive: true }), mkdir(home, { recursive: true })]);
    process.env.HOME = home;

    const result = await inspectPiuEnvironment(
      { agentDirectory, cwd },
      {
        credentials: new InMemoryCredentialStore(),
        getSupportedThinkingLevels,
        modelsStore: new InMemoryModelsStore(),
        pi,
      },
    );

    assert.deepEqual(result.resources, { extensions: [], skills: [], packages: [] });
    await assert.rejects(access(agentDirectory), { code: "ENOENT" });
  } finally {
    if (previousHome === undefined) delete process.env.HOME;
    else process.env.HOME = previousHome;
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

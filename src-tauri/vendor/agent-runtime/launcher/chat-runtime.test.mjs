import assert from "node:assert/strict";
import { join } from "node:path";
import test from "node:test";

import { createPiuChatRuntime } from "./chat-runtime.mjs";

function createPiContract() {
  const calls = [];
  const model = { provider: "openai-codex", id: "gpt-5.6-sol" };
  const services = {
    diagnostics: [{ type: "info", message: "fixture" }],
    modelRuntime: {
      getModel(providerId, modelId) {
        calls.push(["getModel", providerId, modelId]);
        return providerId === model.provider && modelId === model.id ? model : undefined;
      },
    },
  };
  const pi = {
    ModelRuntime: {
      async create(options) {
        calls.push(["modelRuntime", options]);
        return services.modelRuntime;
      },
    },
    SessionManager: {
      continueRecent() {
        throw new Error("Più must never select the most recent session");
      },
      create(cwd, sessionDirectory) {
        calls.push(["createSession", cwd, sessionDirectory]);
        return { kind: "new", cwd, sessionDirectory };
      },
      open(path, sessionDirectory) {
        calls.push(["openSession", path, sessionDirectory]);
        return { kind: "existing", path, sessionDirectory };
      },
    },
    SettingsManager: {
      create(cwd, agentDirectory, options) {
        calls.push(["settings", cwd, agentDirectory, options]);
        return { cwd, agentDirectory, options };
      },
    },
    async createAgentSessionServices(options) {
      calls.push(["services", options]);
      return services;
    },
    async createAgentSessionFromServices(options) {
      calls.push(["session", options]);
      return { session: { id: "session" } };
    },
    async createAgentSessionRuntime(factory, options) {
      calls.push(["runtime", options]);
      return factory({
        cwd: options.cwd,
        agentDir: options.agentDir,
        sessionManager: options.sessionManager,
      });
    },
  };
  return { calls, model, pi };
}

const paths = {
  cwd: "/private/tmp/piu/worktrees/chat-1",
  agentDirectory: "/Users/test/Library/Application Support/ch.emin.piu/agent",
  sessionDirectory: "/Users/test/Library/Application Support/ch.emin.piu/sessions",
  extensionPaths: ["/private/tmp/piu/worktrees/chat-1/.pi/extensions/review.mjs"],
  skillPaths: [
    "/Applications/Più.app/Contents/Resources/agent-runtime/skills",
    "/private/tmp/piu/worktrees/chat-1/.pi/skills",
  ],
};

test("a new chat uses the exact app directories and explicit resource paths", async () => {
  const { calls, model, pi } = createPiContract();
  const credentials = { read: async () => undefined };
  const createNewSessionManager = async ({ cwd, sessionDirectory, SessionManager }) => {
    assert.equal(SessionManager, pi.SessionManager);
    calls.push(["createSession", cwd, sessionDirectory]);
    return { kind: "new", cwd, sessionDirectory };
  };

  const result = await createPiuChatRuntime(
    {
      ...paths,
      modelId: "gpt-5.6-sol",
      modelProvider: "openai-codex",
      thinkingLevel: "medium",
    },
    { credentials, createNewSessionManager, pi },
  );

  assert.deepEqual(result.session, { id: "session" });
  assert.deepEqual(
    calls.find(([kind]) => kind === "modelRuntime"),
    [
      "modelRuntime",
      { credentials, modelsPath: join(paths.agentDirectory, "models.json") },
    ],
  );
  assert.deepEqual(
    calls.find(([kind]) => kind === "createSession"),
    ["createSession", paths.cwd, paths.sessionDirectory],
  );
  const serviceOptions = calls.find(([kind]) => kind === "services")[1];
  assert.equal(serviceOptions.cwd, paths.cwd);
  assert.equal(serviceOptions.agentDir, paths.agentDirectory);
  assert.deepEqual(serviceOptions.resourceLoaderOptions, {
    additionalExtensionPaths: paths.extensionPaths,
    additionalSkillPaths: paths.skillPaths,
    noExtensions: true,
    noSkills: true,
  });
  assert.deepEqual(serviceOptions.settingsManager.options, { projectTrusted: true });
  const sessionOptions = calls.find(([kind]) => kind === "session")[1];
  assert.equal(sessionOptions.model, model);
  assert.equal(sessionOptions.thinkingLevel, "medium");
});

test("relaunch opens only the exact stored session path", async () => {
  const { calls, pi } = createPiContract();
  const sessionPath = `${paths.sessionDirectory}/exact-session.jsonl`;

  await createPiuChatRuntime(
    {
      ...paths,
      modelId: "gpt-5.6-sol",
      modelProvider: "openai-codex",
      sessionPath,
      thinkingLevel: "xhigh",
    },
    { credentials: {}, pi },
  );

  assert.deepEqual(
    calls.find(([kind]) => kind === "openSession"),
    ["openSession", sessionPath, paths.sessionDirectory],
  );
  assert.equal(
    calls.some(([kind]) => kind === "createSession"),
    false,
  );
  const sessionOptions = calls.find(([kind]) => kind === "session")[1];
  assert.equal("model" in sessionOptions, false);
  assert.equal("thinkingLevel" in sessionOptions, false);
});

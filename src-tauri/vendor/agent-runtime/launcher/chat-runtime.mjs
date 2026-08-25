import { randomUUID } from "node:crypto";
import { mkdir, open, unlink } from "node:fs/promises";
import { isAbsolute, join } from "node:path";

function requireAbsolute(name, value) {
  if (typeof value !== "string" || !isAbsolute(value)) {
    throw new Error(`${name} must be an absolute path`);
  }
}

function validate(config) {
  requireAbsolute("cwd", config.cwd);
  requireAbsolute("agent directory", config.agentDirectory);
  requireAbsolute("session directory", config.sessionDirectory);
  if (config.sessionPath !== undefined) requireAbsolute("session path", config.sessionPath);
  if (!Array.isArray(config.skillPaths)) throw new Error("skill paths must be an array");
  for (const skillPath of config.skillPaths) requireAbsolute("skill path", skillPath);
  if (!config.modelProvider || !config.modelId) throw new Error("model route is required");
}

export async function createPersistedSessionManager({ cwd, sessionDirectory, SessionManager }) {
  await mkdir(sessionDirectory, { mode: 0o700, recursive: true });
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const sessionPath = join(sessionDirectory, `piu-${randomUUID()}.jsonl`);
    let handle;
    try {
      handle = await open(sessionPath, "wx", 0o600);
    } catch (error) {
      if (error?.code === "EEXIST") continue;
      throw error;
    }
    try {
      await handle.close();
    } catch (error) {
      await unlink(sessionPath).catch(() => undefined);
      throw error;
    }
    try {
      return SessionManager.open(sessionPath, sessionDirectory, cwd);
    } catch (error) {
      await unlink(sessionPath).catch(() => undefined);
      throw error;
    }
  }
  throw new Error("could not allocate a unique Pi session file");
}

export async function createPiuChatRuntime(
  config,
  { credentials, createNewSessionManager = createPersistedSessionManager, pi },
) {
  validate(config);
  const modelRuntime = await pi.ModelRuntime.create({
    credentials,
    modelsPath: join(config.agentDirectory, "models.json"),
  });
  const initialSessionManager = config.sessionPath
    ? pi.SessionManager.open(config.sessionPath, config.sessionDirectory)
    : await createNewSessionManager({
        cwd: config.cwd,
        sessionDirectory: config.sessionDirectory,
        SessionManager: pi.SessionManager,
      });

  const createRuntime = async ({ cwd, agentDir, sessionManager, sessionStartEvent }) => {
    const settingsManager = pi.SettingsManager.create(cwd, agentDir, {
      projectTrusted: true,
    });
    const services = await pi.createAgentSessionServices({
      cwd,
      agentDir,
      settingsManager,
      modelRuntime,
      resourceLoaderOptions: {
        additionalSkillPaths: config.skillPaths,
        noSkills: true,
      },
    });
    const sessionOptions = {
      services,
      sessionManager,
      sessionStartEvent,
    };
    const restoringInitialSession =
      config.sessionPath !== undefined && sessionManager === initialSessionManager;
    if (!restoringInitialSession) {
      const model = services.modelRuntime.getModel(config.modelProvider, config.modelId);
      if (!model) throw new Error("selected model is unavailable in the pinned Pi runtime");
      sessionOptions.model = model;
      sessionOptions.thinkingLevel = config.thinkingLevel;
    }
    const result = await pi.createAgentSessionFromServices(sessionOptions);
    return {
      ...result,
      services,
      diagnostics: services.diagnostics,
    };
  };

  return pi.createAgentSessionRuntime(createRuntime, {
    cwd: config.cwd,
    agentDir: config.agentDirectory,
    sessionManager: initialSessionManager,
  });
}

const FLAGS = new Map([
  ["--cwd", "cwd"],
  ["--agent-dir", "agentDirectory"],
  ["--session-dir", "sessionDirectory"],
  ["--session-path", "sessionPath"],
  ["--credential-lock-dir", "credentialLockDirectory"],
  ["--model-provider", "modelProvider"],
  ["--model-id", "modelId"],
  ["--thinking-level", "thinkingLevel"],
]);

const REQUIRED_FLAGS = [
  "--cwd",
  "--agent-dir",
  "--session-dir",
  "--credential-lock-dir",
  "--model-provider",
  "--model-id",
  "--thinking-level",
];

export const MAX_ENVIRONMENT_PREFERENCES_BYTES = 256 * 1024;

function takeValue(arguments_, index, flag) {
  const value = arguments_[index + 1];
  if (value === undefined || value.startsWith("--")) {
    throw new Error(`missing value for ${flag}`);
  }
  return value;
}

export function parseChatLauncherArguments(arguments_) {
  const result = { extensionPaths: [], skillPaths: [] };
  const seen = new Set();

  for (let index = 0; index < arguments_.length; index += 2) {
    const flag = arguments_[index];
    const value = takeValue(arguments_, index, flag);

    if (flag === "--skill") {
      result.skillPaths.push(value);
      continue;
    }
    if (flag === "--extension") {
      result.extensionPaths.push(value);
      continue;
    }

    const property = FLAGS.get(flag);
    if (!property) throw new Error(`unknown flag ${flag}`);
    if (seen.has(flag)) throw new Error(`duplicate flag ${flag}`);
    seen.add(flag);
    result[property] = value;
  }

  for (const flag of REQUIRED_FLAGS) {
    if (!seen.has(flag)) throw new Error(`missing required flag ${flag}`);
  }

  return result;
}

export function parseAuthLauncherArguments(arguments_) {
  const requiredFlag = "--credential-lock-dir";
  let credentialLockDirectory;

  for (let index = 0; index < arguments_.length; index += 2) {
    const flag = arguments_[index];
    const value = takeValue(arguments_, index, flag);
    if (flag !== requiredFlag) throw new Error(`unknown flag ${flag}`);
    if (credentialLockDirectory !== undefined) throw new Error(`duplicate flag ${flag}`);
    credentialLockDirectory = value;
  }

  if (credentialLockDirectory === undefined) {
    throw new Error(`missing required flag ${requiredFlag}`);
  }
  return { credentialLockDirectory };
}

export function parseEnvironmentLauncherArguments(arguments_) {
  const flags = new Map([
    ["--cwd", "cwd"],
    ["--agent-dir", "agentDirectory"],
    ["--credential-lock-dir", "credentialLockDirectory"],
    ["--resource-preferences", "resourcePreferences"],
  ]);
  const result = {};
  const seen = new Set();

  for (let index = 0; index < arguments_.length; index += 2) {
    const flag = arguments_[index];
    const value = takeValue(arguments_, index, flag);
    const property = flags.get(flag);
    if (!property) throw new Error(`unknown flag ${flag}`);
    if (seen.has(flag)) throw new Error(`duplicate flag ${flag}`);
    seen.add(flag);
    result[property] = value;
  }

  for (const flag of flags.keys()) {
    if (!seen.has(flag)) throw new Error(`missing required flag ${flag}`);
  }
  result.resourcePreferences = parseEnvironmentResourcePreferences(result.resourcePreferences);
  return result;
}

function parseEnvironmentResourcePreferences(serialized) {
  if (Buffer.byteLength(serialized) > MAX_ENVIRONMENT_PREFERENCES_BYTES) {
    throw new Error("environment resource preferences exceed the input limit");
  }
  let parsed;
  try {
    parsed = JSON.parse(serialized);
  } catch {
    throw new Error("environment resource preferences must be valid JSON");
  }
  if (
    parsed === null ||
    typeof parsed !== "object" ||
    Array.isArray(parsed) ||
    Object.keys(parsed).sort().join(",") !== "global,project" ||
    !Array.isArray(parsed.global) ||
    !Array.isArray(parsed.project)
  ) {
    throw new Error("environment resource preferences have an invalid shape");
  }
  for (const [scope, records] of [
    ["global", parsed.global],
    ["project", parsed.project],
  ]) {
    const seen = new Set();
    for (const record of records) {
      if (
        record === null ||
        typeof record !== "object" ||
        Array.isArray(record) ||
        Object.keys(record).sort().join(",") !== "enabled,id,kind" ||
        (record.kind !== "extension" && record.kind !== "package") ||
        typeof record.id !== "string" ||
        record.id.length === 0 ||
        typeof record.enabled !== "boolean"
      ) {
        throw new Error(`environment ${scope} resource preference has an invalid shape`);
      }
      const identity = `${record.kind}\0${record.id}`;
      if (seen.has(identity)) {
        throw new Error(`duplicate environment ${scope} resource preference`);
      }
      seen.add(identity);
    }
  }
  return parsed;
}

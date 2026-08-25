import { inspectPiuEnvironment } from "./environment-inspection.mjs";
import { parseEnvironmentLauncherArguments } from "./launcher-arguments.mjs";
import { createRuntimeCredentials } from "./runtime-credentials.mjs";

export const MAX_ENVIRONMENT_SNAPSHOT_BYTES = 4 * 1024 * 1024;

export async function runEnvironmentLauncher(
  arguments_,
  {
    ai,
    inspectEnvironment = inspectPiuEnvironment,
    locking,
    pi,
    write = (record) => process.stdout.write(record),
  },
) {
  const config = parseEnvironmentLauncherArguments(arguments_);
  const credentials = createRuntimeCredentials({
    credentialLockDirectory: config.credentialLockDirectory,
    lock: locking.default.lock,
  });
  const snapshot = await inspectEnvironment(config, {
    credentials,
    getSupportedThinkingLevels: ai.getSupportedThinkingLevels,
    modelsStore: new ai.InMemoryModelsStore(),
    pi,
  });
  const record = `${JSON.stringify(snapshot)}\n`;
  if (Buffer.byteLength(record) > MAX_ENVIRONMENT_SNAPSHOT_BYTES) {
    throw new Error("environment snapshot exceeds the output limit");
  }
  write(record);
}

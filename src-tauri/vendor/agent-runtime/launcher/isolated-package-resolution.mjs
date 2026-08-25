import { join } from "node:path";
import { fileURLToPath } from "node:url";

const isolatedNpmRootLauncher = fileURLToPath(new URL("./isolated-npm-root.mjs", import.meta.url));

export function isolatedNpmLookupCommand(agentDirectory) {
  return [
    process.execPath,
    isolatedNpmRootLauncher,
    join(agentDirectory, ".piu-empty-global-npm"),
    "--",
  ];
}

function createScopedMemoryStorage(globalSettings, projectSettings) {
  const values = {
    global: JSON.stringify(globalSettings),
    project: JSON.stringify(projectSettings),
  };
  return {
    withLock(scope, update) {
      const next = update(values[scope]);
      if (next !== undefined) values[scope] = next;
    },
  };
}

export function createChatSettingsManager({ agentDirectory, cwd, SettingsManager }) {
  const fileSettings = SettingsManager.create(cwd, agentDirectory, {
    projectTrusted: true,
  });
  const npmCommand = isolatedNpmLookupCommand(agentDirectory);
  const globalSettings = { ...fileSettings.getGlobalSettings(), npmCommand };
  const projectSettings = { ...fileSettings.getProjectSettings(), npmCommand };
  return SettingsManager.fromStorage(createScopedMemoryStorage(globalSettings, projectSettings), {
    projectTrusted: true,
  });
}

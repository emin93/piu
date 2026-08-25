import { realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, extname, isAbsolute, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const isolatedNpmRootLauncher = fileURLToPath(new URL("./isolated-npm-root.mjs", import.meta.url));

function requireAbsolute(name, value) {
  if (typeof value !== "string" || !isAbsolute(value)) {
    throw new Error(`${name} must be an absolute path`);
  }
}

function validate(config) {
  requireAbsolute("cwd", config.cwd);
  requireAbsolute("agent directory", config.agentDirectory);
  const standaloneAgentDirectory = join(homedir(), ".pi", "agent");
  if (isWithin(config.agentDirectory, standaloneAgentDirectory)) {
    throw new Error("standalone Pi agent directory is not allowed");
  }
}

function pathDisplayName(path) {
  if (basename(path) === "SKILL.md") return basename(dirname(path));
  return basename(path, extname(path));
}

function resourceItem(resource, names = new Map()) {
  return {
    id: resource.path,
    name: names.get(resource.path) ?? pathDisplayName(resource.path),
    path: resource.path,
    enabled: resource.enabled,
    source: resource.metadata.source,
    scope: resource.metadata.scope,
    origin: resource.metadata.origin,
    ...(resource.metadata.baseDir === undefined ? {} : { baseDir: resource.metadata.baseDir }),
  };
}

function configuredPackageItem(configuredPackage) {
  return {
    id: configuredPackage.source,
    name: configuredPackage.source,
    source: configuredPackage.source,
    scope: configuredPackage.scope,
    filtered: configuredPackage.filtered,
    installedPath: configuredPackage.installedPath,
  };
}

function messageFrom(error) {
  return error instanceof Error ? error.message : String(error);
}

function resourcePreferenceMaps(resourcePreferences = { global: [], project: [] }) {
  const maps = {
    global: { extension: new Map(), package: new Map() },
    project: { extension: new Map(), package: new Map() },
  };
  for (const scope of ["global", "project"]) {
    for (const { kind, id, enabled } of resourcePreferences[scope]) {
      maps[scope][kind].set(id, enabled);
    }
  }
  return maps;
}

function effectiveResolvedResourceEnabled(resource, preferences, kind) {
  const packageId = resource.metadata.origin === "package" ? resource.metadata.source : undefined;
  return (
    (kind === "extension" ? preferences.project.extension.get(resource.path) : undefined) ??
    (packageId === undefined ? undefined : preferences.project.package.get(packageId)) ??
    (kind === "extension" ? preferences.global.extension.get(resource.path) : undefined) ??
    (packageId === undefined ? undefined : preferences.global.package.get(packageId)) ??
    resource.enabled
  );
}

function isolatedNpmLookupCommand(agentDirectory) {
  const emptyGlobalRoot = join(agentDirectory, ".piu-empty-global-npm");
  return [process.execPath, isolatedNpmRootLauncher, emptyGlobalRoot, "--"];
}

function captureProviderOwners(extensionsResult, extensionOwners, providerOwners) {
  providerOwners.clear();
  for (const { name, extensionPath } of extensionsResult.runtime.pendingProviderRegistrations) {
    const owner = extensionOwners.get(extensionPath);
    if (owner === undefined) throw new Error(`provider ${name} has no resolved owning extension`);
    providerOwners.set(name, owner);
  }
  for (const { provider, extensionPath } of extensionsResult.runtime
    .pendingNativeProviderRegistrations) {
    const owner = extensionOwners.get(extensionPath);
    if (owner === undefined) {
      throw new Error(`provider ${provider.id} has no resolved owning extension`);
    }
    providerOwners.set(provider.id, owner);
  }
  return extensionsResult;
}

function isWithin(path, root) {
  const pathFromRoot = relative(root, path);
  return pathFromRoot === "" || (!pathFromRoot.startsWith("..") && !isAbsolute(pathFromRoot));
}

async function keepOwnedResources(resources, ownedRoots, canonicalizePath) {
  const decisions = await Promise.all(
    resources.map(async ({ path }) => {
      const canonicalPath = await canonicalizePath(path);
      return ownedRoots.some((root) => isWithin(canonicalPath, root));
    }),
  );
  return resources.filter((_resource, index) => decisions[index]);
}

async function canonicalizeRoot(path, canonicalizePath) {
  try {
    return await canonicalizePath(path);
  } catch (error) {
    if (error?.code === "ENOENT") return path;
    throw error;
  }
}

export async function inspectPiuEnvironment(
  config,
  { canonicalizePath = realpath, credentials, getSupportedThinkingLevels, modelsStore, pi },
) {
  validate(config);
  if (!modelsStore) throw new Error("an in-memory models store is required");
  const diagnostics = [];
  const modelRuntime = await pi.ModelRuntime.create({
    allowModelNetwork: false,
    credentials,
    modelsPath: join(config.agentDirectory, "models.json"),
    modelsStore,
  });
  const settingsManager = pi.SettingsManager.create(config.cwd, config.agentDirectory, {
    projectTrusted: true,
  });
  // `resolve(onMissing)` never installs skipped packages, but Pi also probes legacy global npm.
  // Its documented SettingsManager npmCommand override keeps that lookup deterministic and
  // prevents npm from reading global packages or writing cache/log files beneath the user's HOME.
  settingsManager.applyOverrides({
    npmCommand: isolatedNpmLookupCommand(config.agentDirectory),
  });
  const packageManager = new pi.DefaultPackageManager({
    cwd: config.cwd,
    agentDir: config.agentDirectory,
    settingsManager,
  });
  const missingPackages = new Set();
  let resolved = { extensions: [], skills: [], prompts: [], themes: [] };
  try {
    resolved = await packageManager.resolve(async (source) => {
      missingPackages.add(source);
      return "skip";
    });
  } catch (error) {
    diagnostics.push({
      resourceType: "package",
      type: "error",
      message: messageFrom(error),
    });
  }
  for (const source of missingPackages) {
    diagnostics.push({
      resourceType: "package",
      type: "warning",
      message: "Configured package is unavailable; automatic installation is disabled",
      source,
    });
  }
  for (const { scope, path, error } of settingsManager.drainErrors()) {
    diagnostics.push({
      resourceType: "settings",
      type: "error",
      message: messageFrom(error),
      ...(path === undefined ? {} : { path }),
      scope,
    });
  }

  const ownedRoots = await Promise.all(
    [config.agentDirectory, config.cwd].map((path) => canonicalizeRoot(path, canonicalizePath)),
  );
  resolved = {
    ...resolved,
    extensions: await keepOwnedResources(resolved.extensions, ownedRoots, canonicalizePath),
    skills: await keepOwnedResources(resolved.skills, ownedRoots, canonicalizePath),
  };

  const resourcePreferences = resourcePreferenceMaps(config.resourcePreferences);
  const enabledExtensions = resolved.extensions.filter((resource) =>
    effectiveResolvedResourceEnabled(resource, resourcePreferences, "extension"),
  );
  const enabledSkills = resolved.skills.filter((resource) =>
    effectiveResolvedResourceEnabled(resource, resourcePreferences, "skill"),
  );
  const extensionOwners = new Map(
    enabledExtensions.map(({ path, metadata }) => [
      path,
      {
        extensionId: path,
        scope: metadata.scope,
        ...(metadata.origin === "package" ? { packageId: metadata.source } : {}),
      },
    ]),
  );
  const providerOwners = new Map();
  const services = await pi.createAgentSessionServices({
    cwd: config.cwd,
    agentDir: config.agentDirectory,
    modelRuntime,
    settingsManager: pi.SettingsManager.inMemory({}, { projectTrusted: true }),
    resourceLoaderOptions: {
      additionalExtensionPaths: enabledExtensions.map(({ path }) => path),
      additionalSkillPaths: enabledSkills.map(({ path }) => path),
      noContextFiles: true,
      noExtensions: true,
      noPromptTemplates: true,
      noSkills: true,
      noThemes: true,
      extensionsOverride: (extensionsResult) =>
        captureProviderOwners(extensionsResult, extensionOwners, providerOwners),
    },
  });
  for (const diagnostic of services.diagnostics) {
    diagnostics.push({ resourceType: "runtime", ...diagnostic });
  }
  for (const { path, error } of services.resourceLoader.getExtensions().errors) {
    diagnostics.push({
      resourceType: "extension",
      type: "error",
      message: messageFrom(error),
      path,
    });
  }
  const loadedSkills = services.resourceLoader.getSkills();
  for (const diagnostic of loadedSkills.diagnostics) {
    diagnostics.push({ resourceType: "skill", ...diagnostic });
  }
  const skillNames = new Map(
    (loadedSkills.skills ?? []).map((skill) => [skill.filePath, skill.name]),
  );

  const availableModels = await services.modelRuntime.getAvailable();
  const modelError = services.modelRuntime.getError();
  if (modelError) {
    diagnostics.push({ resourceType: "model", type: "warning", message: modelError });
  }

  return {
    modelRoutes: availableModels.map((model) => {
      const owner = providerOwners.get(model.provider);
      return {
        provider: model.provider,
        id: model.id,
        name: model.name,
        acceptsImages: model.input?.includes("image") ?? false,
        thinkingLevels: getSupportedThinkingLevels(model),
        scope: owner?.scope ?? "user",
        ...(owner === undefined
          ? {}
          : {
              owner: {
                extensionId: owner.extensionId,
                ...(owner.packageId === undefined ? {} : { packageId: owner.packageId }),
              },
            }),
      };
    }),
    resources: {
      extensions: resolved.extensions.map((resource) => resourceItem(resource)),
      skills: resolved.skills.map((resource) => resourceItem(resource, skillNames)),
      packages: packageManager.listConfiguredPackages().map(configuredPackageItem),
    },
    diagnostics,
  };
}

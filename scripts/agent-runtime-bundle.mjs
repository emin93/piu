import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { createWriteStream } from "node:fs";
import {
  access,
  constants,
  copyFile,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
} from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const vendorRoot = join(repositoryRoot, "src-tauri", "vendor", "agent-runtime");
const defaultLockPath = join(vendorRoot, "runtime-lock.json");
const sourcePackagePath = join(vendorRoot, "package.json");
const sourcePackageLockPath = join(vendorRoot, "package-lock.json");
const launcherSourceRoot = join(vendorRoot, "launcher");
const licenseRoot = join(repositoryRoot, "docs", "licenses");
export const defaultRuntimeRoot = join(vendorRoot, "runtime");

const exactVersion = /^\d+\.\d+\.\d+$/;
const gitCommit = /^[a-f0-9]{40}$/;
const sha256 = /^[a-f0-9]{64}$/;
const sha512Integrity = /^sha512-[A-Za-z0-9+/]+={0,2}$/;

function requireMatch(value, pattern, description) {
  if (typeof value !== "string" || !pattern.test(value)) {
    throw new Error(`Runtime lock requires ${description}`);
  }
}

export function validateRuntimeLock(lock) {
  if (lock?.schemaVersion !== 1) {
    throw new Error("Runtime lock requires schema version 1");
  }
  if (lock.target !== "darwin-arm64") {
    throw new Error("Runtime lock requires the darwin-arm64 target");
  }
  requireMatch(lock.node?.version, exactVersion, "a pinned Node version");
  requireMatch(lock.node?.archiveSha256, sha256, "the Node archive SHA-256");
  requireMatch(lock.node?.licenseSha256, sha256, "the Node license SHA-256");
  requireMatch(lock.pi?.commit, gitCommit, "a full Pi commit");
  requireMatch(lock.pi?.licenseSha256, sha256, "the Pi license SHA-256");

  const codingAgent = lock.pi?.packages?.["@earendil-works/pi-coding-agent"];
  requireMatch(codingAgent?.version, exactVersion, "a pinned Pi coding-agent version");
  requireMatch(codingAgent?.integrity, sha512Integrity, "the Pi coding-agent SRI");

  const piAi = lock.pi?.packages?.["@earendil-works/pi-ai"];
  requireMatch(piAi?.version, exactVersion, "a pinned Pi AI version");
  requireMatch(piAi?.integrity, sha512Integrity, "the Pi AI SRI");

  const properLockfile = lock.pi?.packages?.["proper-lockfile"];
  requireMatch(properLockfile?.version, exactVersion, "a pinned proper-lockfile version");
  requireMatch(properLockfile?.integrity, sha512Integrity, "the proper-lockfile SRI");
}

export async function loadRuntimeLock(lockPath = defaultLockPath) {
  const lock = JSON.parse(await readFile(lockPath, "utf8"));
  validateRuntimeLock(lock);
  return lock;
}

export async function sha256File(path) {
  const hash = createHash("sha256");
  hash.update(await readFile(path));
  return hash.digest("hex");
}

async function pathExists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function assertExecutable(path, description) {
  try {
    await access(path, constants.X_OK);
  } catch (error) {
    throw new Error(`${description} is missing or not executable: ${path}`, { cause: error });
  }
}

async function assertSha256(path, expected, description) {
  const actual = await sha256File(path);
  if (actual !== expected) {
    throw new Error(`${description} has SHA-256 ${actual}; expected ${expected}`);
  }
}

async function run(command, arguments_, options = {}) {
  try {
    return await execFileAsync(command, arguments_, {
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
      ...options,
    });
  } catch (error) {
    const detail = error.stderr?.trim() || error.stdout?.trim() || error.message;
    throw new Error(`${command} failed: ${detail}`, { cause: error });
  }
}

function lockedPackage(packageLock, packageName) {
  const entry = packageLock.packages?.[`node_modules/${packageName}`];
  if (!entry) throw new Error(`npm lock does not contain ${packageName}`);
  return entry;
}

async function verifyNpmLock(lock, packageRoot) {
  const sourcePackage = await readFile(sourcePackagePath);
  const sourcePackageLock = await readFile(sourcePackageLockPath);
  const runtimePackage = await readFile(join(packageRoot, "package.json"));
  const runtimePackageLock = await readFile(join(packageRoot, "package-lock.json"));
  if (!runtimePackage.equals(sourcePackage) || !runtimePackageLock.equals(sourcePackageLock)) {
    throw new Error("Bundled Pi package metadata differs from the committed npm lock");
  }

  const packageJson = JSON.parse(sourcePackage.toString("utf8"));
  const packageLock = JSON.parse(sourcePackageLock.toString("utf8"));
  if (packageLock.lockfileVersion !== 3) {
    throw new Error(
      `Bundled Pi requires npm lockfile version 3, received ${packageLock.lockfileVersion}`,
    );
  }

  for (const [packagePath, packageEntry] of Object.entries(packageLock.packages)) {
    if (!packagePath) continue;
    requireMatch(packageEntry.version, exactVersion, `an exact npm version for ${packagePath}`);
    requireMatch(packageEntry.integrity, sha512Integrity, `npm SRI for ${packagePath}`);
    if (!packageEntry.resolved?.startsWith("https://registry.npmjs.org/")) {
      throw new Error(`${packagePath} is not locked to the npm registry`);
    }
  }

  for (const [packageName, expected] of Object.entries(lock.pi.packages)) {
    if (packageJson.dependencies?.[packageName] !== expected.version) {
      throw new Error(`${packageName} is not an exact direct runtime dependency`);
    }
    const packageEntry = lockedPackage(packageLock, packageName);
    if (
      packageEntry.version !== expected.version ||
      packageEntry.integrity !== expected.integrity
    ) {
      throw new Error(`${packageName} does not match the pinned version and SRI`);
    }
    const installedPackage = JSON.parse(
      await readFile(join(packageRoot, "node_modules", packageName, "package.json"), "utf8"),
    );
    if (installedPackage.version !== expected.version) {
      throw new Error(`${packageName} installed version is ${installedPackage.version}`);
    }
  }
}

async function directoryDigest(root) {
  const hash = createHash("sha256");
  async function visit(directory, relativeDirectory) {
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      const relativePath = join(relativeDirectory, entry.name);
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await visit(path, relativePath);
      else if (entry.isFile()) {
        hash.update(relativePath);
        hash.update("\0");
        hash.update(await readFile(path));
        hash.update("\0");
      } else {
        throw new Error(`Launcher source must contain only files and directories: ${path}`);
      }
    }
  }
  await visit(root, "");
  return hash.digest("hex");
}

async function verifyLaunchers(packageRoot) {
  const runtimeLaunchers = join(packageRoot, "launcher");
  const sourceExists = await pathExists(launcherSourceRoot);
  const runtimeExists = await pathExists(runtimeLaunchers);
  if (sourceExists !== runtimeExists) {
    throw new Error("Bundled launcher files differ from the committed launcher source");
  }
  if (
    sourceExists &&
    (await directoryDigest(launcherSourceRoot)) !== (await directoryDigest(runtimeLaunchers))
  ) {
    throw new Error("Bundled launcher files differ from the committed launcher source");
  }
}

async function findNativeAddons(directory) {
  const addons = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) addons.push(...(await findNativeAddons(path)));
    else if (entry.isFile() && entry.name.endsWith(".node")) addons.push(path);
  }
  return addons;
}

async function nativeAddonKind(path) {
  return (await run("/usr/bin/file", ["-b", path])).stdout;
}

async function pruneUnsupportedNativeAddons(packageRoot) {
  for (const addon of await findNativeAddons(join(packageRoot, "node_modules"))) {
    const kind = await nativeAddonKind(addon);
    if (!kind.includes("Mach-O") || !kind.includes("arm64")) {
      await rm(addon, { force: true });
    }
  }
}

export async function verifyAgentRuntime({ runtimeRoot = defaultRuntimeRoot } = {}) {
  const lock = await loadRuntimeLock();
  const nodeRoot = join(runtimeRoot, "node");
  const nodeExecutable = join(nodeRoot, "bin", "node");
  const packageRoot = join(runtimeRoot, "pi");
  await assertExecutable(nodeExecutable, "Bundled Node executable");

  const { stdout: executableKind } = await run("/usr/bin/file", ["-b", nodeExecutable]);
  if (!executableKind.includes("Mach-O") || !executableKind.includes("arm64")) {
    throw new Error(`Bundled Node is not an arm64 Mach-O executable: ${executableKind.trim()}`);
  }

  const childEnvironment = { LC_ALL: "C", PATH: "/usr/bin:/bin" };
  const { stdout: nodeVersion } = await run(nodeExecutable, ["--version"], {
    cwd: runtimeRoot,
    env: childEnvironment,
  });
  if (nodeVersion.trim() !== `v${lock.node.version}`) {
    throw new Error(`Bundled Node reported ${nodeVersion.trim()}; expected v${lock.node.version}`);
  }
  const { stdout: nodeArchitecture } = await run(
    nodeExecutable,
    ["--print", "`${process.platform}-${process.arch}`"],
    { cwd: runtimeRoot, env: childEnvironment },
  );
  if (nodeArchitecture.trim() !== lock.target) {
    throw new Error(`Bundled Node reported ${nodeArchitecture.trim()}; expected ${lock.target}`);
  }

  await assertSha256(join(nodeRoot, "LICENSE"), lock.node.licenseSha256, "Bundled Node LICENSE");
  await assertSha256(join(packageRoot, "LICENSE"), lock.pi.licenseSha256, "Bundled Pi LICENSE");
  await verifyNpmLock(lock, packageRoot);
  await verifyLaunchers(packageRoot);

  const npmCli = join(nodeRoot, "lib", "node_modules", "npm", "bin", "npm-cli.js");
  await run(nodeExecutable, [npmCli, "ls", "--all", "--omit=dev"], {
    cwd: packageRoot,
    env: childEnvironment,
  });

  const publicExportProbe = [
    'const agent = await import("@earendil-works/pi-coding-agent");',
    'const ai = await import("@earendil-works/pi-ai");',
    'const locking = await import("proper-lockfile");',
    'if (typeof agent.createAgentSessionRuntime !== "function") throw new Error("missing createAgentSessionRuntime");',
    'if (typeof agent.runRpcMode !== "function") throw new Error("missing runRpcMode");',
    'if (typeof agent.ModelRuntime !== "function") throw new Error("missing ModelRuntime");',
    'if (typeof ai.createModels !== "function") throw new Error("missing pi-ai createModels");',
    'if (typeof locking.default?.lock !== "function") throw new Error("missing proper-lockfile lock");',
  ].join("\n");
  await run(nodeExecutable, ["--input-type=module", "--eval", publicExportProbe], {
    cwd: packageRoot,
    env: childEnvironment,
  });

  for (const addon of await findNativeAddons(join(packageRoot, "node_modules"))) {
    const addonKind = await nativeAddonKind(addon);
    if (!addonKind.includes("Mach-O") || !addonKind.includes("arm64")) {
      throw new Error(`Bundled native addon is not arm64: ${addon}`);
    }
  }

  return {
    nodeExecutable,
    nodeVersion: lock.node.version,
    packageRoot,
    piCommit: lock.pi.commit,
    piVersion: lock.pi.packages["@earendil-works/pi-coding-agent"].version,
  };
}

async function downloadVerifiedArchive(lock, cacheRoot) {
  await mkdir(cacheRoot, { recursive: true });
  const archivePath = join(cacheRoot, lock.node.archive);
  if (await pathExists(archivePath)) {
    try {
      await assertSha256(archivePath, lock.node.archiveSha256, "Cached Node archive");
      return archivePath;
    } catch {
      await rm(archivePath, { force: true });
    }
  }

  const partialPath = `${archivePath}.partial-${process.pid}`;
  await rm(partialPath, { force: true });
  const response = await fetch(lock.node.archiveUrl, { redirect: "follow" });
  if (!response.ok || !response.body) {
    throw new Error(`Could not download pinned Node archive: HTTP ${response.status}`);
  }
  try {
    await pipeline(
      Readable.fromWeb(response.body),
      createWriteStream(partialPath, { mode: 0o644 }),
    );
    await assertSha256(partialPath, lock.node.archiveSha256, "Downloaded Node archive");
    await rename(partialPath, archivePath);
  } catch (error) {
    await rm(partialPath, { force: true });
    throw error;
  }
  return archivePath;
}

async function installRuntime(lock, stagingRoot, cacheRoot) {
  const archivePath = await downloadVerifiedArchive(lock, cacheRoot);
  const extractionRoot = join(stagingRoot, "extracted");
  const runtimeRoot = join(stagingRoot, "runtime");
  const nodeRoot = join(runtimeRoot, "node");
  const packageRoot = join(runtimeRoot, "pi");
  await mkdir(extractionRoot, { recursive: true });
  await mkdir(packageRoot, { recursive: true });
  await run("/usr/bin/tar", ["-xf", archivePath, "-C", extractionRoot]);

  const extractedNodeRoot = join(extractionRoot, `node-v${lock.node.version}-${lock.target}`);
  if (!(await pathExists(extractedNodeRoot))) {
    throw new Error(`Node archive did not contain node-v${lock.node.version}-${lock.target}`);
  }
  await rename(extractedNodeRoot, nodeRoot);
  await copyFile(sourcePackagePath, join(packageRoot, "package.json"));
  await copyFile(sourcePackageLockPath, join(packageRoot, "package-lock.json"));
  await copyFile(join(licenseRoot, "Pi-MIT.txt"), join(packageRoot, "LICENSE"));
  if (await pathExists(launcherSourceRoot)) {
    await cp(launcherSourceRoot, join(packageRoot, "launcher"), { recursive: true });
  }

  const nodeExecutable = join(nodeRoot, "bin", "node");
  const npmCli = join(nodeRoot, "lib", "node_modules", "npm", "bin", "npm-cli.js");
  const npmCache = join(cacheRoot, "npm");
  await mkdir(npmCache, { recursive: true });
  await run(
    nodeExecutable,
    [npmCli, "ci", "--omit=dev", "--ignore-scripts", "--no-audit", "--no-fund", "--prefer-offline"],
    {
      cwd: packageRoot,
      env: { ...process.env, PATH: "/usr/bin:/bin", npm_config_cache: npmCache },
    },
  );
  await pruneUnsupportedNativeAddons(packageRoot);
  await verifyAgentRuntime({ runtimeRoot });
  return runtimeRoot;
}

export async function provisionAgentRuntime({
  runtimeRoot = defaultRuntimeRoot,
  cacheRoot = join(vendorRoot, ".cache"),
} = {}) {
  if (process.platform !== "darwin" || process.arch !== "arm64") {
    throw new Error("The bundled agent runtime must be provisioned on Apple Silicon macOS");
  }

  if (await pathExists(runtimeRoot)) {
    try {
      return await verifyAgentRuntime({ runtimeRoot });
    } catch {
      // A stale generated runtime is replaced only after its successor verifies.
    }
  }

  const lock = await loadRuntimeLock();
  const runtimeParent = dirname(runtimeRoot);
  await mkdir(runtimeParent, { recursive: true });
  const stagingRoot = await mkdtemp(join(runtimeParent, ".agent-runtime-staging-"));
  const previousRoot = `${runtimeRoot}.previous-${process.pid}`;
  try {
    const stagedRuntime = await installRuntime(lock, stagingRoot, cacheRoot);
    const hadPrevious = await pathExists(runtimeRoot);
    if (hadPrevious) await rename(runtimeRoot, previousRoot);
    try {
      await rename(stagedRuntime, runtimeRoot);
    } catch (error) {
      if (hadPrevious) await rename(previousRoot, runtimeRoot);
      throw error;
    }
    await rm(previousRoot, { force: true, recursive: true });
    return await verifyAgentRuntime({ runtimeRoot });
  } finally {
    await rm(stagingRoot, { force: true, recursive: true });
  }
}

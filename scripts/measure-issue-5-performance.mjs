import { spawn, spawnSync } from "node:child_process";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const resultPrefix = "PIU_ISSUE_5_PERFORMANCE:";
const resultPath = resolve("work/issue-5-performance-result.json");
const tracePath = resolve("work/issue-5-animation-hitches.trace");
const executable = resolve(
  "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Più.app/Contents/MacOS/piu",
);
const appData = await mkdtemp(join(tmpdir(), "piu-issue-5-performance-"));
const originalClipboard = spawnSync("pbpaste", { encoding: null }).stdout;
let app;
let trace;

async function stopChild(child, signal) {
  if (!child || child.exitCode !== null) return;
  child.kill(signal);
  await Promise.race([
    new Promise((resolveExit) => child.once("exit", resolveExit)),
    new Promise((resolveTimeout) => setTimeout(resolveTimeout, 2_000)),
  ]);
}

function run(command, arguments_) {
  const result = spawnSync(command, arguments_, { encoding: "utf8", stdio: "inherit" });
  if (result.status !== 0) throw new Error(`${command} exited with ${String(result.status)}`);
}

async function clipboardResult(timeoutMs) {
  const startedAt = performance.now();
  while (performance.now() - startedAt < timeoutMs) {
    const value = spawnSync("pbpaste", { encoding: "utf8" }).stdout;
    if (value.startsWith(resultPrefix)) return JSON.parse(value.slice(resultPrefix.length));
    await new Promise((resolveWait) => setTimeout(resolveWait, 100));
  }
  throw new Error("The packaged performance harness did not publish a result within 45 seconds");
}

async function foregroundProcess(pid) {
  const script = `tell application "System Events" to set frontmost of first process whose unix id is ${String(pid)} to true`;
  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (spawnSync("osascript", ["-e", script]).status === 0) return;
    await new Promise((resolveWait) => setTimeout(resolveWait, 100));
  }
  throw new Error("Could not foreground the packaged Più benchmark window");
}

try {
  await mkdir(resolve("work"), { recursive: true });
  await rm(tracePath, { force: true, recursive: true });
  spawnSync("pbcopy", { input: "" });
  run("npm", [
    "exec",
    "tauri",
    "build",
    "--",
    "--target",
    "aarch64-apple-darwin",
    "--bundles",
    "app",
    "--config",
    "scripts/performance/issue-5.tauri.conf.json",
  ]);

  app = spawn(executable, [], {
    env: { ...process.env, PATH: "/usr/bin:/bin", PIU_TEST_APP_DATA_DIR: appData },
    stdio: "ignore",
  });
  await foregroundProcess(app.pid);
  await new Promise((resolveWait) => setTimeout(resolveWait, 250));
  trace = spawn(
    "xcrun",
    [
      "xctrace",
      "record",
      "--template",
      "Animation Hitches",
      "--attach",
      String(app.pid),
      "--time-limit",
      "15s",
      "--no-prompt",
      "--output",
      tracePath,
    ],
    { stdio: "ignore" },
  );

  const report = await clipboardResult(45_000);
  if (report.error) throw new Error(report.error);
  await writeFile(resultPath, `${JSON.stringify(report, null, 2)}\n`);
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  process.stdout.write(`RESULT_PATH=${resultPath}\nTRACE_PATH=${tracePath}\n`);
} finally {
  await stopChild(trace, "SIGINT");
  await stopChild(app, "SIGTERM");
  spawnSync("pbcopy", { input: originalClipboard });
  await rm(appData, { force: true, recursive: true });
  run("npm", ["run", "build"]);
}

import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawn } from "node:child_process";

const executable = resolve(
  "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Più.app/Contents/MacOS/piu",
);
const appData = await mkdtemp(join(tmpdir(), "piu-packaged-smoke-"));
const startedAt = performance.now();
const output = [];
let child;
let readinessTimeout;

try {
  child = spawn(executable, [], {
    cwd: dirname(executable),
    env: {
      ...process.env,
      PIU_TEST_APP_DATA_DIR: appData,
      RUST_LOG: "piu=info",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  const ready = new Promise((resolveReady, rejectReady) => {
    const observe = (chunk) => {
      const text = chunk.toString();
      output.push(text);
      if (text.includes("piu_shell_ready")) resolveReady();
    };
    child.stdout.on("data", observe);
    child.stderr.on("data", observe);
    child.once("error", rejectReady);
    child.once("exit", (code, signal) => {
      rejectReady(
        new Error(
          `Packaged Più exited before readiness (code ${String(code)}, signal ${String(signal)})`,
        ),
      );
    });
  });
  const timeout = new Promise((_, rejectTimeout) => {
    readinessTimeout = setTimeout(
      () => rejectTimeout(new Error("Packaged Più did not become ready within 15s")),
      15_000,
    );
  });

  await Promise.race([ready, timeout]);
  clearTimeout(readinessTimeout);
  console.log(`PACKAGED_LAUNCH_MS=${Math.round(performance.now() - startedAt)}`);
} catch (error) {
  throw new Error(`${error.message}\n${output.join("")}`, { cause: error });
} finally {
  clearTimeout(readinessTimeout);
  if (child && child.exitCode === null) child.kill("SIGTERM");
  await rm(appData, { recursive: true, force: true });
}

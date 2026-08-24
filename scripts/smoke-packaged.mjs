import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";

const executable = resolve(
  "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Più.app/Contents/MacOS/piu",
);
const appData = await mkdtemp(join(tmpdir(), "piu-packaged-smoke-"));
const gitSmoke = await mkdtemp(join(tmpdir(), "piu-packaged-git-smoke-"));
const appContents = resolve(dirname(executable), "..");
const gitRoot = join(appContents, "Resources", "git");
const gitExecutable = join(gitRoot, "bin", "git");
const gitEnvironment = {
  HOME: join(gitSmoke, "home"),
  PATH: "/usr/bin:/bin",
  LC_ALL: "C",
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_TERMINAL_PROMPT: "0",
  GIT_EXEC_PATH: join(gitRoot, "libexec", "git-core"),
  GIT_TEMPLATE_DIR: join(gitRoot, "share", "git-core", "templates"),
};
const startedAt = performance.now();
const output = [];
let child;
let readinessTimeout;

try {
  const git = (...arguments_) => {
    const result = spawnSync(gitExecutable, arguments_, {
      env: gitEnvironment,
      encoding: "utf8",
    });
    if (result.status !== 0) {
      throw new Error(
        `Packaged Git failed (${arguments_.join(" ")}): ${result.stderr || result.error}`,
      );
    }
    return result.stdout.trim();
  };
  if (git("version") !== "git version 2.55.0") {
    throw new Error("Packaged Git did not report the pinned 2.55.0 version");
  }
  git("init", "--quiet", "--bare", join(gitSmoke, "remote.git"));
  git("init", "--quiet", join(gitSmoke, "source"));
  git(
    "-C",
    join(gitSmoke, "source"),
    "-c",
    "user.name=Più",
    "-c",
    "user.email=piu@example.invalid",
    "commit",
    "--quiet",
    "--allow-empty",
    "-m",
    "initial",
  );
  git("-C", join(gitSmoke, "source"), "push", "--quiet", join(gitSmoke, "remote.git"), "HEAD:main");
  git("init", "--quiet", join(gitSmoke, "client"));
  git("-C", join(gitSmoke, "client"), "fetch", "--quiet", join(gitSmoke, "remote.git"), "main");
  git(
    "-C",
    join(gitSmoke, "client"),
    "worktree",
    "add",
    "--quiet",
    "--detach",
    join(gitSmoke, "worktree"),
    "FETCH_HEAD",
  );
  console.log("PACKAGED_GIT_VERSION=2.55.0");

  child = spawn(executable, [], {
    cwd: dirname(executable),
    env: {
      ...process.env,
      PATH: "/usr/bin:/bin",
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
  await rm(gitSmoke, { recursive: true, force: true });
}

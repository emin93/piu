import { createInterface } from "node:readline";

import { createAuthInteraction, runCodexLogin } from "./auth-interaction.mjs";
import { parseAuthLauncherArguments } from "./launcher-arguments.mjs";
import { createRuntimeCredentials } from "./runtime-credentials.mjs";

process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_TELEMETRY = "0";

function writeRecord(record) {
  process.stdout.write(`${JSON.stringify(record)}\n`);
}

async function main() {
  const [pi, locking] = await Promise.all([
    import("@earendil-works/pi-coding-agent"),
    import("proper-lockfile"),
  ]);
  const config = parseAuthLauncherArguments(process.argv.slice(2));
  const credentials = createRuntimeCredentials({
    credentialLockDirectory: config.credentialLockDirectory,
    lock: locking.default.lock,
  });
  const modelRuntime = await pi.ModelRuntime.create({ credentials, modelsPath: null });
  const protocol = createAuthInteraction({ emit: writeRecord });
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
  let finished = false;

  input.on("line", (line) => {
    try {
      protocol.accept(JSON.parse(line));
    } catch {
      protocol.cancel(new Error("Authentication protocol failed"));
    }
  });
  input.on("close", () => {
    if (!finished) protocol.cancel();
  });

  const status = await runCodexLogin({ modelRuntime, protocol, emit: writeRecord });
  finished = true;
  input.close();
  if (status !== "complete") process.exitCode = 1;
}

main().catch(() => {
  process.stderr.write("Più authentication helper failed to start.\n");
  process.exitCode = 1;
});

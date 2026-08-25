import { createPiuChatRuntime } from "./chat-runtime.mjs";
import { parseChatLauncherArguments } from "./launcher-arguments.mjs";
import { createRuntimeCredentials } from "./runtime-credentials.mjs";

process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_TELEMETRY = "0";
process.env.PI_OFFLINE = "1";

async function main() {
  const [pi, locking] = await Promise.all([
    import("@earendil-works/pi-coding-agent"),
    import("proper-lockfile"),
  ]);
  const config = parseChatLauncherArguments(process.argv.slice(2));
  const credentials = createRuntimeCredentials({
    credentialLockDirectory: config.credentialLockDirectory,
    lock: locking.default.lock,
  });
  const runtime = await createPiuChatRuntime(config, { credentials, pi });
  await pi.runRpcMode(runtime);
}

main().catch(() => {
  process.stderr.write("Più agent runtime failed to start.\n");
  process.exitCode = 1;
});

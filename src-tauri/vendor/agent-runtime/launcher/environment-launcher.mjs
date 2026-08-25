import { runEnvironmentLauncher } from "./environment-launcher-runtime.mjs";

process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_TELEMETRY = "0";

async function main() {
  const [pi, ai, locking] = await Promise.all([
    import("@earendil-works/pi-coding-agent"),
    import("@earendil-works/pi-ai"),
    import("proper-lockfile"),
  ]);
  await runEnvironmentLauncher(process.argv.slice(2), { ai, locking, pi });
}

main().catch(() => {
  process.stderr.write("Più environment inspection failed.\n");
  process.exitCode = 1;
});

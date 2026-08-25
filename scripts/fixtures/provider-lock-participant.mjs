import { pathToFileURL } from "node:url";

function requiredEnvironment(name) {
  const value = process.env[name];
  if (!value) throw new Error(`missing ${name}`);
  return value;
}

function emit(record) {
  process.stdout.write(`${JSON.stringify(record)}\n`);
}

async function main() {
  const providerLockUrl = pathToFileURL(requiredEnvironment("PIU_PROVIDER_LOCK_MODULE")).href;
  const properLockfileUrl = pathToFileURL(requiredEnvironment("PIU_PROPER_LOCKFILE_MODULE")).href;
  const [{ createProviderLock }, locking] = await Promise.all([
    import(providerLockUrl),
    import(properLockfileUrl),
  ]);
  const withProviderLock = createProviderLock({
    lock: locking.default.lock,
    lockRoot: requiredEnvironment("PIU_LOCK_ROOT"),
    retryDelayMs: 10,
  });

  emit({ type: "waiting", pid: process.pid });
  await withProviderLock("openai-codex", undefined, async () => {
    emit({ type: "acquired", pid: process.pid });
    if (requiredEnvironment("PIU_LOCK_MODE") !== "hold") return;
    process.stdin.resume();
    await new Promise((resolve) => process.stdin.once("end", resolve));
  });
  emit({ type: "released", pid: process.pid });
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
});

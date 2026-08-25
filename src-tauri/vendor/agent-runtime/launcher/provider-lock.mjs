import { createHash } from "node:crypto";
import { mkdir, open } from "node:fs/promises";
import { join } from "node:path";

const PROVIDER_ID = /^[a-z0-9][a-z0-9._-]*$/;

function waitForRetry(milliseconds, signal) {
  signal?.throwIfAborted();
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(finish, milliseconds);
    function finish() {
      signal?.removeEventListener("abort", abort);
      resolve();
    }
    function abort() {
      clearTimeout(timeout);
      signal?.removeEventListener("abort", abort);
      reject(signal.reason);
    }
    signal?.addEventListener("abort", abort, { once: true });
  });
}

export function createProviderLock({ lock, lockRoot, retryDelayMs = 25 }) {
  return async function withProviderLock(providerId, signal, operation) {
    signal?.throwIfAborted();
    if (!PROVIDER_ID.test(providerId)) throw new Error("invalid credential provider");
    await mkdir(lockRoot, { mode: 0o700, recursive: true });
    const digest = createHash("sha256").update(providerId).digest("hex");
    const target = join(lockRoot, `${digest}.credential`);
    const targetFile = await open(target, "a", 0o600);
    await targetFile.close();

    let release;
    while (!release) {
      signal?.throwIfAborted();
      try {
        release = await lock(target, {
          realpath: false,
          retries: 0,
          stale: 5_000,
          update: 1_000,
        });
      } catch (error) {
        if (error?.code !== "ELOCKED") throw error;
        await waitForRetry(retryDelayMs, signal);
      }
    }

    try {
      signal?.throwIfAborted();
      return await operation();
    } finally {
      await release();
    }
  };
}

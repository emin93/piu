import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { createProviderLock } from "./provider-lock.mjs";

function createLockAdapter() {
  const held = new Set();
  return async (path) => {
    if (held.has(path)) {
      const error = new Error("already locked");
      error.code = "ELOCKED";
      throw error;
    }
    held.add(path);
    return async () => {
      held.delete(path);
    };
  };
}

async function fixture(run) {
  const lockRoot = await mkdtemp(join(tmpdir(), "piu-provider-lock-"));
  try {
    await run(
      createProviderLock({
        lock: createLockAdapter(),
        lockRoot,
        retryDelayMs: 1,
      }),
    );
  } finally {
    await rm(lockRoot, { recursive: true, force: true });
  }
}

test("operations for one provider are serialized", async () => {
  await fixture(async (withProviderLock) => {
    const entries = [];
    let releaseFirst;
    let firstStarted;
    const firstGate = new Promise((resolve) => {
      releaseFirst = resolve;
    });
    const firstStartedGate = new Promise((resolve) => {
      firstStarted = resolve;
    });
    const first = withProviderLock("openai-codex", undefined, async () => {
      entries.push("first:start");
      firstStarted();
      await firstGate;
      entries.push("first:end");
    });
    await firstStartedGate;
    const second = withProviderLock("openai-codex", undefined, async () => {
      entries.push("second");
    });

    await new Promise((resolve) => setTimeout(resolve, 5));
    assert.deepEqual(entries, ["first:start"]);
    releaseFirst();
    await Promise.all([first, second]);
    assert.deepEqual(entries, ["first:start", "first:end", "second"]);
  });
});

test("a waiter can abort without disturbing the current owner", async () => {
  await fixture(async (withProviderLock) => {
    let releaseFirst;
    const firstGate = new Promise((resolve) => {
      releaseFirst = resolve;
    });
    const first = withProviderLock("openai-codex", undefined, () => firstGate);
    const controller = new AbortController();
    const waiting = withProviderLock("openai-codex", controller.signal, async () => {
      throw new Error("an aborted waiter must not enter");
    });

    controller.abort(new Error("cancelled while waiting"));
    await assert.rejects(waiting, /cancelled while waiting/);
    releaseFirst();
    await first;
  });
});

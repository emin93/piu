import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { access, mkdtemp, readdir, rm, utimes } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

const runtimeRoot = resolve("src-tauri/vendor/agent-runtime/runtime");
const nodeExecutable = join(runtimeRoot, "node", "bin", "node");
const providerLockModule = join(runtimeRoot, "pi", "launcher", "provider-lock.mjs");
const properLockfileModule = join(runtimeRoot, "pi", "node_modules", "proper-lockfile", "index.js");
const participantFixture = resolve("scripts/fixtures/provider-lock-participant.mjs");

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}

function startParticipant(lockRoot, mode) {
  const child = spawn(nodeExecutable, [participantFixture], {
    env: {
      LC_ALL: "C",
      PATH: "/usr/bin:/bin",
      PIU_LOCK_MODE: mode,
      PIU_LOCK_ROOT: lockRoot,
      PIU_PROPER_LOCKFILE_MODULE: properLockfileModule,
      PIU_PROVIDER_LOCK_MODULE: providerLockModule,
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const records = [];
  const waiters = new Set();
  let buffered = "";
  let stderr = "";

  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    buffered += chunk;
    for (;;) {
      const newline = buffered.indexOf("\n");
      if (newline === -1) break;
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      const record = JSON.parse(line);
      records.push(record);
      for (const waiter of waiters) {
        if (!waiter.predicate(record)) continue;
        clearTimeout(waiter.timeout);
        waiters.delete(waiter);
        waiter.resolve(record);
      }
    }
  });

  const exited = new Promise((resolveExit) => {
    child.once("exit", (code, signal) => resolveExit({ code, signal }));
  });

  return {
    child,
    exited,
    records,
    stderr: () => stderr,
    waitForRecord(predicate, description) {
      const existing = records.find(predicate);
      if (existing) return Promise.resolve(existing);
      return new Promise((resolveRecord, reject) => {
        const waiter = {
          predicate,
          resolve: resolveRecord,
          timeout: setTimeout(() => {
            waiters.delete(waiter);
            reject(new Error(`${description} timed out: ${stderr}`));
          }, 10_000),
        };
        waiter.timeout.unref();
        waiters.add(waiter);
      });
    },
  };
}

test("the pinned provider lock recovers after processes die while holding and waiting", async () => {
  await Promise.all([
    access(nodeExecutable),
    access(providerLockModule),
    access(properLockfileModule),
    access(participantFixture),
  ]);
  const lockRoot = await mkdtemp(join(tmpdir(), "piu-provider-lock-contract-"));
  const participants = [];

  try {
    const holder = startParticipant(lockRoot, "hold");
    participants.push(holder);
    await holder.waitForRecord((record) => record.type === "acquired", "holder acquisition");

    const killedWaiter = startParticipant(lockRoot, "once");
    participants.push(killedWaiter);
    await killedWaiter.waitForRecord((record) => record.type === "waiting", "first waiter start");
    await delay(100);
    assert.equal(
      killedWaiter.records.some((record) => record.type === "acquired"),
      false,
    );
    killedWaiter.child.kill("SIGKILL");
    assert.deepEqual(await killedWaiter.exited, { code: null, signal: "SIGKILL" });

    const survivor = startParticipant(lockRoot, "once");
    participants.push(survivor);
    await survivor.waitForRecord((record) => record.type === "waiting", "surviving waiter start");
    await delay(100);
    assert.equal(
      survivor.records.some((record) => record.type === "acquired"),
      false,
    );

    holder.child.kill("SIGKILL");
    assert.deepEqual(await holder.exited, { code: null, signal: "SIGKILL" });

    const lockEntries = (await readdir(lockRoot)).filter((entry) => entry.endsWith(".lock"));
    assert.equal(lockEntries.length, 1);
    const staleTime = new Date(Date.now() - 10_000);
    await utimes(join(lockRoot, lockEntries[0]), staleTime, staleTime);

    await survivor.waitForRecord((record) => record.type === "acquired", "stale-lock recovery");
    assert.deepEqual(await survivor.exited, { code: 0, signal: null });
    assert.equal(survivor.stderr(), "");
  } finally {
    for (const participant of participants) {
      if (participant.child.exitCode === null && participant.child.signalCode === null) {
        participant.child.kill("SIGKILL");
      }
    }
    await Promise.allSettled(participants.map((participant) => participant.exited));
    await rm(lockRoot, { recursive: true, force: true });
  }
});

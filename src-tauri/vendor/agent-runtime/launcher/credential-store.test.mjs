import assert from "node:assert/strict";
import test from "node:test";

import { KeychainCredentialStore } from "./credential-store.mjs";

function clone(value) {
  return value === undefined ? undefined : structuredClone(value);
}

function createMemoryKeychain(initial = {}) {
  const credentials = new Map(Object.entries(initial));
  return {
    async delete(providerId) {
      credentials.delete(providerId);
    },
    async exists(providerId) {
      return credentials.has(providerId);
    },
    async read(providerId) {
      return clone(credentials.get(providerId));
    },
    async write(providerId, credential) {
      credentials.set(providerId, clone(credential));
    },
  };
}

function createProviderLocks() {
  const tails = new Map();
  return async (providerId, _signal, operation) => {
    const previous = tails.get(providerId) ?? Promise.resolve();
    let release;
    const current = new Promise((resolve) => {
      release = resolve;
    });
    tails.set(
      providerId,
      previous.then(() => current),
    );
    await previous;
    try {
      return await operation();
    } finally {
      release();
      if (tails.get(providerId) === current) tails.delete(providerId);
    }
  };
}

const oauth = (access, refresh = "refresh-1") => ({
  type: "oauth",
  access,
  refresh,
  expires: 1,
});

test("concurrent refreshes observe one serialized credential update", async () => {
  const keychain = createMemoryKeychain({ "openai-codex": oauth("expired") });
  const store = new KeychainCredentialStore({
    keychain,
    providerCredentialTypes: { "openai-codex": "oauth" },
    withProviderLock: createProviderLocks(),
  });
  let refreshes = 0;
  const refresh = async (current) => {
    if (current?.access !== "expired") return undefined;
    refreshes += 1;
    await Promise.resolve();
    return oauth("fresh", "rotated-refresh");
  };

  const [first, second] = await Promise.all([
    store.modify("openai-codex", refresh),
    store.modify("openai-codex", refresh),
  ]);

  assert.equal(refreshes, 1);
  assert.deepEqual(first, oauth("fresh", "rotated-refresh"));
  assert.deepEqual(second, oauth("fresh", "rotated-refresh"));
  assert.deepEqual(await store.read("openai-codex"), oauth("fresh", "rotated-refresh"));
});

test("concurrent reads return complete credentials without taking the modification lock", async () => {
  let activeReads = 0;
  let maximumActiveReads = 0;
  let readsStarted = 0;
  let resolveReadsStarted;
  let releaseReads;
  const readsStartedGate = new Promise((resolve) => {
    resolveReadsStarted = resolve;
  });
  const readGate = new Promise((resolve) => {
    releaseReads = resolve;
  });
  const keychain = {
    async delete() {},
    async exists() {
      return true;
    },
    async read() {
      activeReads += 1;
      maximumActiveReads = Math.max(maximumActiveReads, activeReads);
      readsStarted += 1;
      if (readsStarted === 2) resolveReadsStarted();
      await readGate;
      activeReads -= 1;
      return oauth("current", "rotated-refresh");
    },
    async write() {},
  };
  const store = new KeychainCredentialStore({
    keychain,
    providerCredentialTypes: { "openai-codex": "oauth" },
    withProviderLock: async () => {
      throw new Error("reads must not take the modification lock");
    },
  });

  const reads = Promise.all([store.read("openai-codex"), store.read("openai-codex")]);
  await readsStartedGate;
  assert.equal(maximumActiveReads, 2);
  releaseReads();

  assert.deepEqual(await reads, [
    oauth("current", "rotated-refresh"),
    oauth("current", "rotated-refresh"),
  ]);
});

test("listing reports metadata without reading credential values", async () => {
  let secretReads = 0;
  const keychain = {
    async delete() {},
    async exists(providerId) {
      return providerId === "openai-codex";
    },
    async read() {
      secretReads += 1;
      throw new Error("list must not resolve a secret");
    },
    async write() {},
  };
  const store = new KeychainCredentialStore({
    keychain,
    providerCredentialTypes: {
      "openai-codex": "oauth",
      anthropic: "api_key",
    },
    withProviderLock: createProviderLocks(),
  });

  assert.deepEqual(await store.list(), [{ providerId: "openai-codex", type: "oauth" }]);
  assert.equal(secretReads, 0);
});

test("delete waits for an in-flight modification and leaves no credential", async () => {
  const keychain = createMemoryKeychain({ "openai-codex": oauth("expired") });
  const store = new KeychainCredentialStore({
    keychain,
    providerCredentialTypes: { "openai-codex": "oauth" },
    withProviderLock: createProviderLocks(),
  });
  let releaseRefresh;
  const refreshGate = new Promise((resolve) => {
    releaseRefresh = resolve;
  });
  const modifying = store.modify("openai-codex", async () => {
    await refreshGate;
    return oauth("fresh");
  });
  const deleting = store.delete("openai-codex");

  releaseRefresh();
  await Promise.all([modifying, deleting]);

  assert.equal(await store.read("openai-codex"), undefined);
});

test("aborted operations never enter the storage seam", async () => {
  let touched = false;
  const keychain = {
    async delete() {
      touched = true;
    },
    async exists() {
      touched = true;
      return false;
    },
    async read() {
      touched = true;
    },
    async write() {
      touched = true;
    },
  };
  const store = new KeychainCredentialStore({
    keychain,
    providerCredentialTypes: { "openai-codex": "oauth" },
    withProviderLock: createProviderLocks(),
  });
  const controller = new AbortController();
  controller.abort(new Error("cancelled"));

  await assert.rejects(store.read("openai-codex", { signal: controller.signal }), /cancelled/);
  await assert.rejects(store.list({ signal: controller.signal }), /cancelled/);
  await assert.rejects(
    store.modify("openai-codex", async () => oauth("fresh"), {
      signal: controller.signal,
    }),
    /cancelled/,
  );
  await assert.rejects(store.delete("openai-codex", { signal: controller.signal }), /cancelled/);
  assert.equal(touched, false);
});

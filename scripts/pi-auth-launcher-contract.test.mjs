import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

const runtimeRoot = resolve("src-tauri/vendor/agent-runtime/runtime");
const nodeExecutable = join(runtimeRoot, "node", "bin", "node");
const launcher = join(runtimeRoot, "pi", "launcher", "auth-launcher.mjs");

function withTimeout(promise, description, diagnostics) {
  let timeout;
  const expired = new Promise((_, reject) => {
    timeout = setTimeout(
      () => reject(new Error(`${description} timed out: ${diagnostics()}`)),
      10_000,
    );
    timeout.unref();
  });
  return Promise.race([promise, expired]).finally(() => clearTimeout(timeout));
}

function startAuthenticationHelper(credentialLockDirectory) {
  const child = spawn(
    nodeExecutable,
    [launcher, "--credential-lock-dir", credentialLockDirectory],
    {
      cwd: join(runtimeRoot, "pi"),
      env: {
        HOME: process.env.HOME,
        LC_ALL: "C",
        PATH: "/usr/bin:/bin",
        PI_OAUTH_CALLBACK_HOST: "127.0.0.1",
      },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  let stderr = "";
  let buffered = "";
  const records = [];
  const waiters = new Set();
  const exited = new Promise((resolveExit) => {
    child.once("exit", (code, signal) => resolveExit({ code, signal }));
  });

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
        waiters.delete(waiter);
        waiter.resolve(record);
      }
    }
  });

  return {
    child,
    exited,
    records,
    stderr: () => stderr,
    send(record) {
      child.stdin.write(`${JSON.stringify(record)}\n`);
    },
    waitForRecord(predicate, description) {
      const existing = records.find(predicate);
      if (existing) return Promise.resolve(existing);
      return withTimeout(
        new Promise((resolveRecord) => {
          const waiter = { predicate, resolve: resolveRecord };
          waiters.add(waiter);
        }),
        description,
        () => stderr,
      );
    },
    waitForExit(description) {
      return withTimeout(exited, description, () => stderr);
    },
  };
}

async function enterBrowserLogin(helper) {
  const methodPrompt = await helper.waitForRecord(
    (record) => record.type === "auth_prompt" && record.id === "auth-1",
    "authentication prompt",
  );
  assert.deepEqual(methodPrompt, {
    type: "auth_prompt",
    id: "auth-1",
    prompt: {
      type: "select",
      message: "Select OpenAI Codex login method:",
      options: [
        { id: "browser", label: "Browser login (default)" },
        { id: "device_code", label: "Device code login (headless)" },
      ],
    },
  });
  helper.send({ type: "auth_prompt_response", id: "auth-1", value: "browser" });

  const codePrompt = await helper.waitForRecord(
    (record) => record.type === "auth_prompt" && record.id === "auth-2",
    "manual authentication prompt",
  );
  const authEvent = helper.records.find(
    (record) => record.type === "auth_event" && record.event?.type === "auth_url",
  );
  assert.equal(
    authEvent?.event.instructions,
    "A browser window should open. Complete login to finish.",
  );
  const authorizationUrl = new URL(authEvent.event.url);
  const callbackUrl = new URL(authorizationUrl.searchParams.get("redirect_uri"));
  const expectedState = authorizationUrl.searchParams.get("state");
  assert.equal(callbackUrl.protocol, "http:");
  assert.equal(callbackUrl.hostname, "localhost");
  assert.equal(callbackUrl.pathname, "/auth/callback");
  assert.equal(typeof expectedState, "string");
  assert.equal(expectedState.length > 0, true);
  assert.deepEqual(codePrompt, {
    type: "auth_prompt",
    id: "auth-2",
    prompt: {
      type: "manual_code",
      message:
        "Complete login in your browser, or paste the authorization code / redirect URL here:",
      placeholder: callbackUrl.toString(),
    },
  });
  return { callbackUrl, expectedState };
}

async function cancelAuthentication(helper) {
  const recordCount = helper.records.length;
  helper.send({ type: "auth_cancel" });
  assert.deepEqual(await helper.waitForExit("authentication helper shutdown"), {
    code: 1,
    signal: null,
  });
  assert.deepEqual(helper.records.slice(recordCount), [
    { type: "auth_prompt_cancelled", id: "auth-2" },
    { type: "auth_cancelled" },
  ]);
  assert.equal(helper.stderr(), "");
  assert.equal(JSON.stringify(helper.records).includes("access_token"), false);
  assert.equal(JSON.stringify(helper.records).includes("refresh_token"), false);
}

async function listen(server, port) {
  await new Promise((resolveListen, reject) => {
    function failed(error) {
      server.off("listening", started);
      reject(error);
    }
    function started() {
      server.off("error", failed);
      resolveListen();
    }
    server.once("error", failed);
    server.once("listening", started);
    server.listen(port, "127.0.0.1");
  });
}

async function close(server) {
  if (!server.listening) return;
  await new Promise((resolveClose, reject) => {
    server.close((error) => (error ? reject(error) : resolveClose()));
  });
}

test("the pinned authentication helper rejects invalid callback state and cancels without secrets", async () => {
  const fixtureRoot = await mkdtemp(join(tmpdir(), "piu-auth-contract-"));
  const credentialLockDirectory = join(fixtureRoot, "credential-locks");
  await mkdir(credentialLockDirectory, { recursive: true });
  const helper = startAuthenticationHelper(credentialLockDirectory);

  try {
    const { callbackUrl, expectedState } = await enterBrowserLogin(helper);
    callbackUrl.searchParams.set("code", "must-not-exchange");
    callbackUrl.searchParams.set("state", `${expectedState}-invalid`);
    const invalidCallback = await fetch(callbackUrl);
    assert.equal(invalidCallback.status, 400);
    assert.match(await invalidCallback.text(), /State mismatch/);
    await cancelAuthentication(helper);
  } finally {
    if (helper.child.exitCode === null && helper.child.signalCode === null) {
      helper.child.kill("SIGKILL");
    }
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

test("the pinned authentication helper retains manual-code recovery when callback bind fails", async () => {
  const fixtureRoot = await mkdtemp(join(tmpdir(), "piu-auth-bind-contract-"));
  const credentialLockDirectory = join(fixtureRoot, "credential-locks");
  await mkdir(credentialLockDirectory, { recursive: true });
  const helpers = [];
  const callbackBlocker = createServer();

  try {
    const discovery = startAuthenticationHelper(credentialLockDirectory);
    helpers.push(discovery);
    const { callbackUrl } = await enterBrowserLogin(discovery);
    await cancelAuthentication(discovery);

    await listen(callbackBlocker, Number(callbackUrl.port));
    assert.equal(callbackBlocker.listening, true);

    const conflicted = startAuthenticationHelper(credentialLockDirectory);
    helpers.push(conflicted);
    const conflictedFlow = await enterBrowserLogin(conflicted);
    assert.equal(conflictedFlow.callbackUrl.origin, callbackUrl.origin);
    assert.equal(callbackBlocker.listening, true);
    await cancelAuthentication(conflicted);
  } finally {
    for (const helper of helpers) {
      if (helper.child.exitCode === null && helper.child.signalCode === null) {
        helper.child.kill("SIGKILL");
      }
    }
    await close(callbackBlocker);
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

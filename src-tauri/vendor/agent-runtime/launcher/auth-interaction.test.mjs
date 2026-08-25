import assert from "node:assert/strict";
import test from "node:test";

import { createAuthInteraction, runCodexLogin } from "./auth-interaction.mjs";

test("relays every public authentication event without extra fields", () => {
  const records = [];
  const protocol = createAuthInteraction({ emit: (record) => records.push(record) });

  protocol.interaction.notify({
    type: "info",
    message: "Choose a sign-in method",
    links: [{ url: "https://example.test/help", label: "Help" }],
    credential: "must-not-cross",
  });
  protocol.interaction.notify({
    type: "auth_url",
    url: "https://example.test/auth",
    instructions: "Continue in the browser",
  });
  protocol.interaction.notify({
    type: "device_code",
    userCode: "ABCD-EFGH",
    verificationUri: "https://example.test/device",
    intervalSeconds: 5,
    expiresInSeconds: 900,
  });
  protocol.interaction.notify({ type: "progress", message: "Waiting" });

  assert.deepEqual(records, [
    {
      type: "auth_event",
      event: {
        type: "info",
        message: "Choose a sign-in method",
        links: [{ url: "https://example.test/help", label: "Help" }],
      },
    },
    {
      type: "auth_event",
      event: {
        type: "auth_url",
        url: "https://example.test/auth",
        instructions: "Continue in the browser",
      },
    },
    {
      type: "auth_event",
      event: {
        type: "device_code",
        userCode: "ABCD-EFGH",
        verificationUri: "https://example.test/device",
        intervalSeconds: 5,
        expiresInSeconds: 900,
      },
    },
    { type: "auth_event", event: { type: "progress", message: "Waiting" } },
  ]);
});

test("correlates prompt responses and strips AbortSignal and unknown fields", async () => {
  const records = [];
  const protocol = createAuthInteraction({ emit: (record) => records.push(record) });
  const answer = protocol.interaction.prompt({
    type: "select",
    message: "Sign in using",
    options: [
      { id: "browser", label: "Browser", description: "Recommended" },
      { id: "device_code", label: "Device code" },
    ],
    signal: new AbortController().signal,
    credential: "must-not-cross",
  });

  assert.deepEqual(records, [
    {
      type: "auth_prompt",
      id: "auth-1",
      prompt: {
        type: "select",
        message: "Sign in using",
        options: [
          { id: "browser", label: "Browser", description: "Recommended" },
          { id: "device_code", label: "Device code" },
        ],
      },
    },
  ]);
  assert.equal(
    protocol.accept({ type: "auth_prompt_response", id: "auth-1", value: "browser" }),
    true,
  );
  assert.equal(await answer, "browser");
});

test("a provider-resolved prompt rejects locally and ignores its raced response", async () => {
  const records = [];
  const promptController = new AbortController();
  const protocol = createAuthInteraction({ emit: (record) => records.push(record) });
  const answer = protocol.interaction.prompt({
    type: "manual_code",
    message: "Paste the callback code",
    signal: promptController.signal,
  });

  promptController.abort(new Error("browser callback won"));
  await assert.rejects(answer, /browser callback won/);
  assert.deepEqual(records.at(-1), { type: "auth_prompt_cancelled", id: "auth-1" });
  assert.equal(
    protocol.accept({ type: "auth_prompt_response", id: "auth-1", value: "late-code" }),
    false,
  );
});

test("a correlated response wins its race with provider abort and leaves later prompts intact", async () => {
  const records = [];
  const promptController = new AbortController();
  const protocol = createAuthInteraction({ emit: (record) => records.push(record) });
  const browserCode = protocol.interaction.prompt({
    type: "manual_code",
    message: "Paste the callback code",
    signal: promptController.signal,
  });

  assert.equal(
    protocol.accept({ type: "auth_prompt_response", id: "auth-1", value: "browser-code" }),
    true,
  );
  promptController.abort(new Error("late provider cancellation"));
  assert.equal(await browserCode, "browser-code");
  assert.equal(
    records.some((record) => record.type === "auth_prompt_cancelled"),
    false,
  );

  const fallback = protocol.interaction.prompt({
    type: "text",
    message: "Use a fallback code",
  });
  assert.equal(
    protocol.accept({ type: "auth_prompt_response", id: "auth-2", value: "fallback-code" }),
    true,
  );
  assert.equal(await fallback, "fallback-code");
});

test("cancelling the helper aborts the login and every pending prompt", async () => {
  const protocol = createAuthInteraction({ emit() {} });
  const answer = protocol.interaction.prompt({ type: "text", message: "Continue" });

  protocol.accept({ type: "auth_cancel" });

  assert.equal(protocol.interaction.signal.aborted, true);
  await assert.rejects(answer, /cancelled/i);
});

test("successful login emits completion without returning or serializing credentials", async () => {
  const records = [];
  const protocol = createAuthInteraction({ emit: (record) => records.push(record) });
  const credential = {
    type: "oauth",
    access: "sensitive-access-token",
    refresh: "sensitive-refresh-token",
    expires: Date.now() + 60_000,
  };
  const modelRuntime = {
    async login(providerId, authType, interaction) {
      assert.equal(providerId, "openai-codex");
      assert.equal(authType, "oauth");
      assert.equal(interaction, protocol.interaction);
      return credential;
    },
  };

  assert.equal(
    await runCodexLogin({ modelRuntime, protocol, emit: (record) => records.push(record) }),
    "complete",
  );
  assert.deepEqual(records, [{ type: "auth_complete" }]);
  assert.equal(JSON.stringify(records).includes("sensitive"), false);
});

test("login failures use a fixed recovery message and never expose provider errors", async () => {
  const records = [];
  const protocol = createAuthInteraction({ emit() {} });
  const modelRuntime = {
    async login() {
      throw new Error("request contained sensitive-access-token");
    },
  };

  assert.equal(
    await runCodexLogin({ modelRuntime, protocol, emit: (record) => records.push(record) }),
    "failed",
  );
  assert.deepEqual(records, [
    {
      type: "auth_failed",
      code: "sign_in_failed",
      message: "Sign-in failed. Try again.",
    },
  ]);
  assert.equal(JSON.stringify(records).includes("sensitive"), false);
});

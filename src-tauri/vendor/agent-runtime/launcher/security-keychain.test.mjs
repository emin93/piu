import assert from "node:assert/strict";
import test from "node:test";

import { createSecurityKeychain } from "./security-keychain.mjs";

const credential = {
  type: "oauth",
  access: "access-secret",
  refresh: "refresh-secret",
  expires: 1,
};

test("writes credentials through stdin without placing secrets in process arguments", async () => {
  const calls = [];
  const keychain = createSecurityKeychain({
    runCommand: async (request) => {
      calls.push(request);
      return { code: 0, stdout: "" };
    },
    service: "ch.emin.piu.test",
  });

  await keychain.write("openai-codex", credential);

  assert.equal(calls.length, 1);
  assert.equal(calls[0].executable, "/usr/bin/security");
  assert.deepEqual(calls[0].args, [
    "add-generic-password",
    "-a",
    "openai-codex",
    "-s",
    "ch.emin.piu.test",
    "-U",
    "-w",
  ]);
  assert.equal(calls[0].args.join(" ").includes("access-secret"), false);
  const serialized = JSON.stringify(credential);
  assert.equal(calls[0].input, `${serialized}\n${serialized}\n`);
});

test("metadata checks never request or return a secret", async () => {
  const calls = [];
  const keychain = createSecurityKeychain({
    runCommand: async (request) => {
      calls.push(request);
      return { code: 0, stdout: "keychain item metadata" };
    },
    service: "ch.emin.piu.test",
  });

  assert.equal(await keychain.exists("openai-codex"), true);
  assert.equal(calls[0].args.includes("-w"), false);
});

test("reads exact JSON and treats an absent item as missing", async () => {
  let response = { code: 0, stdout: `${JSON.stringify(credential)}\n` };
  const keychain = createSecurityKeychain({
    runCommand: async () => response,
    service: "ch.emin.piu.test",
  });

  assert.deepEqual(await keychain.read("openai-codex"), credential);
  response = { code: 44, stdout: "" };
  assert.equal(await keychain.read("openai-codex"), undefined);
});

test("command failures are sanitized before they cross the adapter", async () => {
  const keychain = createSecurityKeychain({
    runCommand: async () => ({
      code: 1,
      stdout: "credential-secret",
      stderr: "credential-secret",
    }),
    service: "ch.emin.piu.test",
  });

  await assert.rejects(keychain.read("openai-codex"), (error) => {
    assert.equal(error.code, "keychainUnavailable");
    assert.equal(error.message.includes("credential-secret"), false);
    return true;
  });
});

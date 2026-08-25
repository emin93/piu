import assert from "node:assert/strict";
import test from "node:test";

import { createSecurityKeychain } from "./security-keychain.mjs";

const enabled = process.platform === "darwin" && process.env.PIU_KEYCHAIN_INTEGRATION === "1";

test(
  "the macOS Keychain adapter writes reads and deletes an isolated credential",
  {
    skip: !enabled,
  },
  async () => {
    const providerId = "openai-codex-test";
    const keychain = createSecurityKeychain({ service: `ch.emin.piu.test.${process.pid}` });
    const credential = {
      type: "oauth",
      access: "integration-access",
      refresh: "integration-refresh",
      expires: 1,
    };
    try {
      await keychain.write(providerId, credential);
      assert.equal(await keychain.exists(providerId), true);
      assert.deepEqual(await keychain.read(providerId), credential);
    } finally {
      await keychain.delete(providerId);
    }
    assert.equal(await keychain.exists(providerId), false);
  },
);

import assert from "node:assert/strict";
import test from "node:test";

import {
  parseAuthLauncherArguments,
  parseChatLauncherArguments,
  parseEnvironmentLauncherArguments,
} from "./launcher-arguments.mjs";

test("parses the fixed internal chat launcher contract", () => {
  assert.deepEqual(
    parseChatLauncherArguments([
      "--cwd",
      "/private/tmp/chat",
      "--agent-dir",
      "/private/tmp/app/agent",
      "--session-dir",
      "/private/tmp/app/sessions",
      "--session-path",
      "/private/tmp/app/sessions/exact.jsonl",
      "--credential-lock-dir",
      "/private/tmp/app/credential-locks",
      "--model-provider",
      "openai-codex",
      "--model-id",
      "gpt-5.6-sol",
      "--thinking-level",
      "xhigh",
      "--skill",
      "/Applications/Più.app/Contents/Resources/skills",
      "--skill",
      "/private/tmp/chat/.pi/skills",
      "--extension",
      "/private/tmp/chat/.pi/extensions/review.mjs",
    ]),
    {
      cwd: "/private/tmp/chat",
      agentDirectory: "/private/tmp/app/agent",
      sessionDirectory: "/private/tmp/app/sessions",
      sessionPath: "/private/tmp/app/sessions/exact.jsonl",
      credentialLockDirectory: "/private/tmp/app/credential-locks",
      modelProvider: "openai-codex",
      modelId: "gpt-5.6-sol",
      thinkingLevel: "xhigh",
      extensionPaths: ["/private/tmp/chat/.pi/extensions/review.mjs"],
      skillPaths: [
        "/Applications/Più.app/Contents/Resources/skills",
        "/private/tmp/chat/.pi/skills",
      ],
    },
  );
});

test("rejects missing values duplicate singleton flags and unknown flags", () => {
  assert.throws(() => parseChatLauncherArguments(["--cwd"]), /missing value/);
  assert.throws(
    () => parseChatLauncherArguments(["--cwd", "/one", "--cwd", "/two"]),
    /duplicate flag/,
  );
  assert.throws(() => parseChatLauncherArguments(["--other", "value"]), /unknown flag/);
});

test("parses the fixed internal authentication launcher contract", () => {
  assert.deepEqual(
    parseAuthLauncherArguments(["--credential-lock-dir", "/private/tmp/app/credential-locks"]),
    { credentialLockDirectory: "/private/tmp/app/credential-locks" },
  );
});

test("authentication arguments reject omitted duplicate and unrelated flags", () => {
  assert.throws(() => parseAuthLauncherArguments([]), /missing required flag/);
  assert.throws(
    () =>
      parseAuthLauncherArguments([
        "--credential-lock-dir",
        "/one",
        "--credential-lock-dir",
        "/two",
      ]),
    /duplicate flag/,
  );
  assert.throws(() => parseAuthLauncherArguments(["--cwd", "/tmp"]), /unknown flag/);
});

test("parses the fixed one-shot environment launcher contract", () => {
  assert.deepEqual(
    parseEnvironmentLauncherArguments([
      "--cwd",
      "/private/tmp/project",
      "--agent-dir",
      "/private/tmp/app/agent",
      "--credential-lock-dir",
      "/private/tmp/app/credential-locks",
    ]),
    {
      cwd: "/private/tmp/project",
      agentDirectory: "/private/tmp/app/agent",
      credentialLockDirectory: "/private/tmp/app/credential-locks",
    },
  );
});

test("environment arguments reject omitted duplicate and unrelated flags", () => {
  assert.throws(() => parseEnvironmentLauncherArguments([]), /missing required flag/);
  assert.throws(
    () =>
      parseEnvironmentLauncherArguments([
        "--cwd",
        "/one",
        "--cwd",
        "/two",
        "--agent-dir",
        "/agent",
        "--credential-lock-dir",
        "/locks",
      ]),
    /duplicate flag/,
  );
  assert.throws(() => parseEnvironmentLauncherArguments(["--session-dir", "/tmp"]), /unknown flag/);
});

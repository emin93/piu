import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

const runtimeRoot = resolve("src-tauri/vendor/agent-runtime/runtime");
const nodeExecutable = join(runtimeRoot, "node", "bin", "node");
const launcher = join(runtimeRoot, "pi", "launcher", "chat-launcher.mjs");

function startChat({
  agentDirectory,
  credentialLockDirectory,
  cwd,
  home,
  sessionDirectory,
  sessionPath,
  skillPaths,
}) {
  const arguments_ = [
    launcher,
    "--cwd",
    cwd,
    "--agent-dir",
    agentDirectory,
    "--session-dir",
    sessionDirectory,
    "--credential-lock-dir",
    credentialLockDirectory,
    "--model-provider",
    "openai-codex",
    "--model-id",
    "gpt-5.6-sol",
    "--thinking-level",
    "xhigh",
  ];
  for (const skillPath of skillPaths) arguments_.push("--skill", skillPath);
  if (sessionPath) arguments_.push("--session-path", sessionPath);
  const child = spawn(nodeExecutable, arguments_, {
    cwd,
    env: {
      HOME: home,
      LC_ALL: "C",
      PATH: "/usr/bin:/bin",
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const pending = new Map();
  const eventWaiters = new Set();
  const observedEvents = [];
  let nextRequest = 0;
  let stdout = "";
  let stderr = "";

  function failPending(error) {
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
    for (const waiter of eventWaiters) waiter.reject(error);
    eventWaiters.clear();
  }

  function observeEvent(record) {
    observedEvents.push(record);
    for (const waiter of eventWaiters) {
      if (!waiter.predicate(record)) continue;
      eventWaiters.delete(waiter);
      clearTimeout(waiter.timeout);
      waiter.resolve(record);
    }
  }

  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
    for (;;) {
      const newline = stdout.indexOf("\n");
      if (newline === -1) break;
      const line = stdout.slice(0, newline);
      stdout = stdout.slice(newline + 1);
      const record = JSON.parse(line);
      if (record.type !== "response" || typeof record.id !== "string") {
        observeEvent(record);
        continue;
      }
      const waiter = pending.get(record.id);
      if (!waiter) continue;
      pending.delete(record.id);
      clearTimeout(waiter.timeout);
      if (record.success) waiter.resolve(record);
      else waiter.reject(new Error(record.error ?? "Pi request failed"));
    }
  });
  child.on("error", failPending);
  child.on("exit", (code, signal) => {
    failPending(
      new Error(
        `Pi launcher exited before responding (code ${String(code)}, signal ${String(signal)}): ${stderr}`,
      ),
    );
  });

  return {
    async request(command) {
      const id = `contract-${++nextRequest}`;
      const response = new Promise((resolveResponse, rejectResponse) => {
        const timeout = setTimeout(() => {
          pending.delete(id);
          rejectResponse(new Error(`Pi launcher timed out handling ${command.type}: ${stderr}`));
        }, 15_000);
        pending.set(id, { reject: rejectResponse, resolve: resolveResponse, timeout });
      });
      child.stdin.write(`${JSON.stringify({ ...command, id })}\n`);
      return response;
    },
    waitForEvent(predicate, description) {
      const observed = observedEvents.find(predicate);
      if (observed) return Promise.resolve(observed);
      return new Promise((resolveEvent, rejectEvent) => {
        const waiter = {
          predicate,
          reject: rejectEvent,
          resolve: resolveEvent,
          timeout: undefined,
        };
        waiter.timeout = setTimeout(() => {
          eventWaiters.delete(waiter);
          rejectEvent(new Error(`Pi launcher timed out waiting for ${description}: ${stderr}`));
        }, 15_000);
        eventWaiters.add(waiter);
      });
    },
    async stop() {
      child.stdin.end();
      const result = await new Promise((resolveExit, rejectExit) => {
        if (child.exitCode !== null) {
          resolveExit({ code: child.exitCode, signal: child.signalCode });
          return;
        }
        const timeout = setTimeout(() => {
          child.kill("SIGKILL");
          rejectExit(new Error(`Pi launcher did not stop after stdin closed: ${stderr}`));
        }, 10_000);
        child.once("exit", (code, signal) => {
          clearTimeout(timeout);
          resolveExit({ code, signal });
        });
      });
      assert.deepEqual(result, { code: 0, signal: null });
      assert.equal(stderr, "");
    },
  };
}

test("the pinned launcher creates and resumes one exact isolated Pi session", async () => {
  await access(nodeExecutable);
  await access(launcher);
  const fixtureRoot = await mkdtemp(join(tmpdir(), "piu-launcher-contract-"));
  const paths = {
    agentDirectory: join(fixtureRoot, "app", "agent"),
    credentialLockDirectory: join(fixtureRoot, "app", "credential-locks"),
    cwd: join(fixtureRoot, "worktree"),
    home: join(fixtureRoot, "home"),
    sessionDirectory: join(fixtureRoot, "app", "sessions"),
    skillPaths: [join(fixtureRoot, "app", "skills")],
  };
  const explicitSkill = join(paths.skillPaths[0], "piu-app-explicit");
  const agentsHomeSkill = join(paths.home, ".agents", "skills", "piu-home-agents");
  const piHomeSkill = join(paths.home, ".pi", "agent", "skills", "piu-home-pi");
  const piHomeExtensionDirectory = join(paths.home, ".pi", "agent", "extensions");
  const projectExtensionDirectory = join(paths.cwd, ".pi", "extensions");
  const projectContextMarker = "PIU_TRUSTED_PROJECT_CONTEXT_9ad61f";
  let chat;

  try {
    await Promise.all([
      mkdir(paths.agentDirectory, { recursive: true }),
      mkdir(paths.credentialLockDirectory, { recursive: true }),
      mkdir(paths.cwd, { recursive: true }),
      mkdir(paths.sessionDirectory, { recursive: true }),
      mkdir(explicitSkill, { recursive: true }),
      mkdir(agentsHomeSkill, { recursive: true }),
      mkdir(piHomeSkill, { recursive: true }),
      mkdir(piHomeExtensionDirectory, { recursive: true }),
      mkdir(projectExtensionDirectory, { recursive: true }),
    ]);
    await Promise.all([
      writeFile(
        join(explicitSkill, "SKILL.md"),
        "---\nname: piu-app-explicit\ndescription: Explicit Più fixture\n---\nUse only in tests.\n",
      ),
      writeFile(
        join(agentsHomeSkill, "SKILL.md"),
        "---\nname: piu-home-agents\ndescription: Must stay isolated\n---\nNever load.\n",
      ),
      writeFile(
        join(piHomeSkill, "SKILL.md"),
        "---\nname: piu-home-pi\ndescription: Must stay isolated\n---\nNever load.\n",
      ),
      writeFile(
        join(piHomeExtensionDirectory, "piu-home-extension.js"),
        'export default function (pi) { pi.registerCommand("piu-home-extension", { description: "Must stay isolated", handler: async () => {} }); }\n',
      ),
      writeFile(join(paths.cwd, "AGENTS.md"), `${projectContextMarker}\n`),
      writeFile(
        join(projectExtensionDirectory, "piu-contract.js"),
        `export default function (pi) {
  pi.registerCommand("piu-contract-event", {
    description: "Exercise Più's pinned public RPC contract",
    handler: async (args, ctx) => {
      pi.sendMessage({
        customType: "piu-contract",
        content: [{ type: "text", text: \`contract:\${args}\` }],
        display: true,
        details: {
          cwd: ctx.cwd,
          projectTrusted: ctx.isProjectTrusted(),
          projectContextLoaded: ctx.getSystemPrompt().includes("${projectContextMarker}"),
        },
      }, { triggerTurn: false });
    },
  });
}
`,
      ),
    ]);

    chat = startChat(paths);
    const initial = (await chat.request({ type: "get_state" })).data;
    assert.equal(initial.model.provider, "openai-codex");
    assert.equal(initial.model.id, "gpt-5.6-sol");
    assert.equal(initial.thinkingLevel, "xhigh");
    assert.equal(typeof initial.sessionId, "string");
    assert.equal(initial.sessionFile.startsWith(`${paths.sessionDirectory}/`), true);
    await access(initial.sessionFile);
    assert.equal((await stat(initial.sessionFile)).mode & 0o777, 0o600);
    const sessionHeader = JSON.parse((await readFile(initial.sessionFile, "utf8")).split("\n")[0]);
    assert.equal(sessionHeader.type, "session");
    assert.equal(sessionHeader.version, 3);
    assert.equal(sessionHeader.id, initial.sessionId);
    assert.equal(typeof sessionHeader.timestamp, "string");
    assert.equal(sessionHeader.cwd, paths.cwd);
    const commands = (await chat.request({ type: "get_commands" })).data.commands;
    assert.equal(
      commands.some((command) => command.name === "skill:piu-app-explicit"),
      true,
    );
    assert.equal(
      commands.some((command) => command.name === "skill:piu-home-agents"),
      false,
    );
    assert.equal(
      commands.some((command) => command.name === "skill:piu-home-pi"),
      false,
    );
    assert.equal(
      commands.some((command) => command.name === "piu-home-extension"),
      false,
    );
    assert.equal(
      commands.some((command) => command.name === "piu-contract-event"),
      true,
    );

    const customMessageEvent = chat.waitForEvent(
      (event) => event.type === "message_end" && event.message?.customType === "piu-contract",
      "the trusted project extension's custom message event",
    );
    await chat.request({ type: "prompt", message: "/piu-contract-event native-rpc" });
    const emittedMessage = (await customMessageEvent).message;
    assert.deepEqual(emittedMessage.content, [{ type: "text", text: "contract:native-rpc" }]);
    assert.deepEqual(emittedMessage.details, {
      cwd: paths.cwd,
      projectTrusted: true,
      projectContextLoaded: true,
    });
    await chat.request({ type: "abort" });

    const queueUpdate = chat.waitForEvent(
      (event) => event.type === "queue_update" && event.steering?.includes("queued guidance"),
      "the native steering queue event",
    );
    await chat.request({ type: "steer", message: "queued guidance" });
    assert.deepEqual((await queueUpdate).steering, ["queued guidance"]);

    const beforeResume = (await chat.request({ type: "get_state" })).data;
    assert.equal(beforeResume.messageCount, initial.messageCount + 1);
    assert.equal(beforeResume.pendingMessageCount, 1);
    const messages = (await chat.request({ type: "get_messages" })).data.messages;
    assert.deepEqual(messages, [emittedMessage]);
    await chat.stop();
    chat = undefined;

    chat = startChat({ ...paths, sessionPath: initial.sessionFile });
    const resumed = (await chat.request({ type: "get_state" })).data;
    assert.equal(resumed.sessionId, initial.sessionId);
    assert.equal(resumed.sessionFile, initial.sessionFile);
    assert.equal(resumed.messageCount, beforeResume.messageCount);
    assert.equal(resumed.pendingMessageCount, 0);
    const resumedMessages = (await chat.request({ type: "get_messages" })).data.messages;
    assert.deepEqual(resumedMessages, messages);
    await chat.stop();
    chat = undefined;
  } finally {
    if (chat) await chat.stop().catch(() => {});
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

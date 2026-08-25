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
  extensionPaths = [],
  home,
  modelId = "gpt-5.6-sol",
  modelProvider = "openai-codex",
  sessionDirectory,
  sessionPath,
  skillPaths,
  thinkingLevel = "xhigh",
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
    modelProvider,
    "--model-id",
    modelId,
    "--thinking-level",
    thinkingLevel,
  ];
  for (const extensionPath of extensionPaths) arguments_.push("--extension", extensionPath);
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
    respondToExtension(response) {
      child.stdin.write(`${JSON.stringify({ type: "extension_ui_response", ...response })}\n`);
    },
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
    async terminate(signal = "SIGKILL") {
      const result = await new Promise((resolveExit) => {
        if (child.exitCode !== null || child.signalCode !== null) {
          resolveExit({ code: child.exitCode, signal: child.signalCode });
          return;
        }
        child.once("exit", (code, exitSignal) => resolveExit({ code, signal: exitSignal }));
        child.kill(signal);
      });
      return result;
    },
  };
}

test("the real pinned Pi process exposes the rich event contract without external inference", async () => {
  const fixtureRoot = await mkdtemp(join(tmpdir(), "piu-event-contract-"));
  const paths = {
    agentDirectory: join(fixtureRoot, "app", "agent"),
    credentialLockDirectory: join(fixtureRoot, "app", "credential-locks"),
    cwd: join(fixtureRoot, "worktree"),
    home: join(fixtureRoot, "home"),
    modelId: "event-matrix",
    modelProvider: "piu-contract",
    sessionDirectory: join(fixtureRoot, "app", "sessions"),
    skillPaths: [],
    thinkingLevel: "high",
  };
  const projectExtensionDirectory = join(paths.cwd, ".pi", "extensions");
  paths.extensionPaths = [join(projectExtensionDirectory, "piu-event-contract.js")];
  let chat;

  try {
    await Promise.all([
      mkdir(paths.agentDirectory, { recursive: true }),
      mkdir(paths.credentialLockDirectory, { recursive: true }),
      mkdir(paths.cwd, { recursive: true }),
      mkdir(paths.sessionDirectory, { recursive: true }),
      mkdir(projectExtensionDirectory, { recursive: true }),
    ]);
    await writeFile(
      join(projectExtensionDirectory, "piu-event-contract.js"),
      `import {
  Type,
  fauxAssistantMessage,
  fauxProvider,
  fauxThinking,
  fauxToolCall,
} from "@earendil-works/pi-ai";

export default function (pi) {
  const provider = fauxProvider({
    provider: "piu-contract",
    models: [
      { id: "event-matrix", name: "Più event matrix", reasoning: true },
      { id: "plain", name: "Più plain model", reasoning: false },
    ],
    tokenSize: { min: 100, max: 100 },
    tokensPerSecond: 5000,
  });
  provider.models[0].thinkingLevelMap = { xhigh: "xhigh", max: "max" };
  provider.setResponses([
    fauxAssistantMessage([
      fauxThinking("contract reasoning"),
      fauxToolCall("piu_contract_tool", { mode: "complete" }, { id: "piu-tool-complete" }),
    ], { stopReason: "toolUse" }),
    fauxAssistantMessage("contract tool finished"),
    fauxAssistantMessage(
      fauxToolCall("piu_contract_tool", { mode: "abort" }, { id: "piu-tool-abort" }),
      { stopReason: "toolUse" },
    ),
    fauxAssistantMessage([], {
      stopReason: "error",
      errorMessage: "contract provider failure",
    }),
    fauxAssistantMessage(fauxThinking(\`process interruption \${"x".repeat(100_000)}\`)),
  ]);
  pi.registerProvider(provider.provider);
  pi.registerTool({
    name: "piu_contract_tool",
    label: "Più contract tool",
    description: "Exercise Pi's public tool lifecycle",
    parameters: Type.Object({ mode: Type.Union([Type.Literal("complete"), Type.Literal("abort")]) }),
    async execute(_toolCallId, params, signal, onUpdate) {
      onUpdate?.({ content: [{ type: "text", text: "tool:running" }], details: { mode: params.mode } });
      if (params.mode === "abort") {
        await new Promise((_resolve, reject) => {
          const abort = () => reject(new Error("contract tool aborted"));
          if (signal?.aborted) abort();
          else signal?.addEventListener("abort", abort, { once: true });
        });
      }
      return {
        content: [{ type: "text", text: \`tool:\${params.mode}\` }],
        details: { mode: params.mode },
      };
    },
  });
  pi.registerCommand("piu-contract-input", {
    description: "Exercise Pi's public RPC extension input protocol",
    handler: async (_args, ctx) => {
      const answer = await ctx.ui.input("Contract input", "Type approved");
      pi.sendMessage({
        customType: "piu-contract-input",
        content: [{ type: "text", text: \`input:\${answer ?? "cancelled"}\` }],
        display: true,
      }, { triggerTurn: false });
    },
  });
}
`,
    );
    chat = startChat(paths);
    const state = (await chat.request({ type: "get_state" })).data;
    assert.equal(state.model.provider, "piu-contract");
    assert.equal(state.model.id, "event-matrix");
    assert.deepEqual((await chat.request({ type: "get_available_thinking_levels" })).data.levels, [
      "off",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
    await chat.request({ type: "set_thinking_level", level: "max" });
    assert.equal((await chat.request({ type: "get_state" })).data.thinkingLevel, "max");
    const availableModels = (await chat.request({ type: "get_available_models" })).data.models;
    assert.equal(
      availableModels.some(({ provider, id }) => provider === "piu-contract" && id === "plain"),
      true,
    );
    await chat.request({ type: "set_model", provider: "piu-contract", modelId: "plain" });
    assert.deepEqual((await chat.request({ type: "get_available_thinking_levels" })).data.levels, [
      "off",
    ]);
    assert.equal((await chat.request({ type: "get_state" })).data.thinkingLevel, "off");
    await chat.request({
      type: "set_model",
      provider: "piu-contract",
      modelId: "event-matrix",
    });
    await chat.request({ type: "set_thinking_level", level: "high" });
    const commands = (await chat.request({ type: "get_commands" })).data.commands;
    assert.equal(
      commands.some((command) => command.name === "piu-contract-input"),
      true,
    );
    const inputRequest = chat.waitForEvent(
      (event) => event.type === "extension_ui_request" && event.method === "input",
      "the public extension input request",
    );
    const inputResult = chat.waitForEvent(
      (event) => event.type === "message_end" && event.message?.customType === "piu-contract-input",
      "the extension's accepted input result",
    );
    const inputPrompt = chat.request({ type: "prompt", message: "/piu-contract-input" });
    const request = await inputRequest;
    assert.equal(request.title, "Contract input");
    chat.respondToExtension({ id: request.id, value: "approved" });
    await inputPrompt;
    assert.deepEqual((await inputResult).message.content, [
      { type: "text", text: "input:approved" },
    ]);

    await chat.request({ type: "set_auto_retry", enabled: false });
    const reasoningDelta = chat.waitForEvent(
      (event) =>
        event.type === "message_update" && event.assistantMessageEvent?.type === "thinking_delta",
      "the provider's real reasoning delta",
    );
    const toolStarted = chat.waitForEvent(
      (event) => event.type === "tool_execution_start" && event.toolName === "piu_contract_tool",
      "the extension tool start",
    );
    const toolUpdated = chat.waitForEvent(
      (event) => event.type === "tool_execution_update" && event.toolCallId === "piu-tool-complete",
      "the extension tool's progressive update",
    );
    const toolEnded = chat.waitForEvent(
      (event) => event.type === "tool_execution_end" && event.toolCallId === "piu-tool-complete",
      "the extension tool result",
    );
    const completedTurn = chat.waitForEvent(
      (event) => event.type === "agent_end" && event.messages?.length > 0,
      "the completed provider turn",
    );
    await chat.request({ type: "prompt", message: "Exercise reasoning and the owned tool" });
    assert.equal((await reasoningDelta).assistantMessageEvent.delta, "contract reasoning");
    assert.equal((await toolStarted).toolCallId, "piu-tool-complete");
    assert.deepEqual((await toolUpdated).partialResult.content, [
      { type: "text", text: "tool:running" },
    ]);
    assert.equal((await toolEnded).isError, false);
    assert.equal((await completedTurn).willRetry, false);

    const ownedToolStarted = chat.waitForEvent(
      (event) => event.type === "tool_execution_start" && event.toolCallId === "piu-tool-abort",
      "the owned tool that remains active until abort",
    );
    const ownedToolEnded = chat.waitForEvent(
      (event) => event.type === "tool_execution_end" && event.toolCallId === "piu-tool-abort",
      "the aborted owned tool result",
    );
    const abortedTurn = chat.waitForEvent(
      (event) =>
        event.type === "agent_end" &&
        event.messages?.some(
          (message) =>
            message.role === "assistant" &&
            message.content?.some((content) => content.id === "piu-tool-abort"),
        ),
      "the aborted agent turn",
    );
    await chat.request({ type: "prompt", message: "Abort the owned tool" });
    await ownedToolStarted;
    await chat.request({ type: "abort" });
    assert.equal((await ownedToolEnded).isError, true);
    assert.equal((await abortedTurn).willRetry, false);

    const failedMessage = chat.waitForEvent(
      (event) =>
        event.type === "message_end" &&
        event.message?.role === "assistant" &&
        event.message.errorMessage === "contract provider failure",
      "the provider's failed assistant message",
    );
    const failedTurn = chat.waitForEvent(
      (event) =>
        event.type === "agent_end" &&
        event.messages?.some((message) => message.errorMessage === "contract provider failure"),
      "the failed agent turn",
    );
    await chat.request({ type: "prompt", message: "Exercise a failed provider turn" });
    assert.equal((await failedMessage).message.stopReason, "error");
    assert.equal((await failedTurn).willRetry, false);

    const beforeInterruption = (await chat.request({ type: "get_state" })).data;
    const interruptedDelta = chat.waitForEvent(
      (event) =>
        event.type === "message_update" &&
        event.assistantMessageEvent?.type === "thinking_delta" &&
        event.assistantMessageEvent.delta.startsWith("process interruption"),
      "the provider delta immediately before process interruption",
    );
    await chat.request({ type: "prompt", message: "Interrupt the real Pi process" });
    await interruptedDelta;
    const interrupted = (await chat.request({ type: "get_state" })).data;
    assert.equal(interrupted.isStreaming, true);
    assert.equal(interrupted.sessionId, beforeInterruption.sessionId);
    assert.deepEqual(await chat.terminate(), { code: null, signal: "SIGKILL" });
    chat = undefined;

    chat = startChat({ ...paths, sessionPath: interrupted.sessionFile });
    const resumed = (await chat.request({ type: "get_state" })).data;
    assert.equal(resumed.sessionId, interrupted.sessionId);
    assert.equal(resumed.sessionFile, interrupted.sessionFile);
    assert.equal(resumed.isStreaming, false);
    assert.equal(resumed.messageCount, beforeInterruption.messageCount + 1);
    const resumedMessages = (await chat.request({ type: "get_messages" })).data.messages;
    assert.equal(resumedMessages.at(-1).role, "user");
    assert.deepEqual(resumedMessages.at(-1).content, [
      { type: "text", text: "Interrupt the real Pi process" },
    ]);
    await assert.rejects(access(join(paths.agentDirectory, "auth.json")), {
      code: "ENOENT",
    });
    await assert.rejects(access(join(paths.home, ".pi", "agent", "auth.json")), {
      code: "ENOENT",
    });
  } finally {
    if (chat) await chat.stop().catch(() => {});
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

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
  paths.extensionPaths = [join(projectExtensionDirectory, "piu-contract.js")];
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

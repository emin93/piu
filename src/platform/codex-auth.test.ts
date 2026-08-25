import type { Event } from "@tauri-apps/api/event";
import { beforeEach, expect, test, vi } from "vitest";

import type { CodexAuthUpdate } from "../generated/CodexAuthUpdate";

import { codexAuthAdapter, type CodexAuthRecord } from "./codex-auth";

const boundary = vi.hoisted(() => ({
  handler: undefined as ((event: Event<unknown>) => void) | undefined,
  invoke: vi.fn(),
  listen: vi.fn(),
  openUrl: vi.fn(),
  order: [] as string[],
  unlisten: vi.fn(),
  writeText: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: boundary.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: boundary.listen }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: boundary.openUrl }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: boundary.writeText }));

beforeEach(() => {
  boundary.handler = undefined;
  boundary.invoke.mockReset();
  boundary.listen.mockReset();
  boundary.openUrl.mockReset();
  boundary.order.length = 0;
  boundary.unlisten.mockReset();
  boundary.writeText.mockReset();
  boundary.listen.mockImplementation(
    (_eventName: string, handler: (event: Event<unknown>) => void): Promise<() => void> => {
      boundary.order.push("listen");
      boundary.handler = handler;
      return Promise.resolve(boundary.unlisten);
    },
  );
  boundary.invoke.mockImplementation((command: string) => {
    boundary.order.push(command);
    return Promise.resolve({ state: "signingIn" });
  });
});

test("sign-in observes the first native update before starting the helper", async () => {
  const receive = vi.fn();
  const firstUpdate: CodexAuthUpdate = {
    type: "auth_event",
    event: {
      type: "info",
      message: "Open the browser to continue.",
      links: [{ label: "Help", url: "https://example.test/help" }],
    },
  };
  boundary.invoke.mockImplementationOnce((command: string) => {
    boundary.order.push(command);
    boundary.handler?.({ payload: firstUpdate } as Event<CodexAuthUpdate>);
    return Promise.resolve({ state: "signingIn" });
  });

  await codexAuthAdapter.connect(receive);

  expect(boundary.order).toEqual(["listen", "start_codex_sign_in"]);
  expect(boundary.listen).toHaveBeenCalledWith("codex-auth://update", expect.any(Function));
  expect(receive).toHaveBeenCalledWith({
    type: "auth_event",
    event: {
      type: "info",
      message: "Open the browser to continue.",
      links: [{ label: "Help", url: "https://example.test/help" }],
    },
  });
});

test("native notifications map nullable generated fields without accepting malformed payloads", async () => {
  const received: CodexAuthRecord[] = [];
  const receive = vi.fn((record: CodexAuthRecord) => received.push(record));
  await codexAuthAdapter.connect(receive);
  const updates: CodexAuthUpdate[] = [
    {
      type: "auth_event",
      event: {
        type: "auth_url",
        url: "https://auth.openai.com/authorize",
        instructions: null,
      },
    },
    {
      type: "auth_event",
      event: {
        type: "device_code",
        userCode: "ABCD-EFGH",
        verificationUri: "https://auth.openai.com/device",
        intervalSeconds: 5,
        expiresInSeconds: null,
      },
    },
    {
      type: "auth_event",
      event: { type: "progress", message: "Waiting for approval…" },
    },
  ];

  for (const update of updates) {
    boundary.handler?.({ payload: update } as Event<CodexAuthUpdate>);
  }
  boundary.handler?.({
    payload: { type: "auth_event", event: { type: "progress" } },
  } as Event<unknown>);

  expect(received).toEqual([
    {
      type: "auth_event",
      event: { type: "auth_url", url: "https://auth.openai.com/authorize" },
    },
    {
      type: "auth_event",
      event: {
        type: "device_code",
        userCode: "ABCD-EFGH",
        verificationUri: "https://auth.openai.com/device",
        intervalSeconds: 5,
      },
    },
    {
      type: "auth_event",
      event: { type: "progress", message: "Waiting for approval…" },
    },
  ]);
});

test("native prompts and terminal outcomes map to the host-independent auth records", async () => {
  const received: CodexAuthRecord[] = [];
  const receive = vi.fn((record: CodexAuthRecord) => received.push(record));
  await codexAuthAdapter.connect(receive);
  const updates: CodexAuthUpdate[] = [
    {
      type: "auth_prompt",
      id: "method",
      prompt: {
        type: "select",
        message: "Choose a sign-in method",
        options: [
          { id: "browser", label: "Browser", description: "Use ChatGPT in your browser" },
          { id: "manual", label: "Manual code", description: null },
        ],
      },
    },
    {
      type: "auth_prompt",
      id: "email",
      prompt: { type: "text", message: "Email", placeholder: null },
    },
    {
      type: "auth_prompt",
      id: "password",
      prompt: { type: "secret", message: "Password", placeholder: "Password" },
    },
    {
      type: "auth_prompt",
      id: "callback",
      prompt: { type: "manual_code", message: "Paste the callback code", placeholder: null },
    },
    { type: "auth_prompt_cancelled", id: "callback" },
    { type: "auth_complete" },
    { type: "auth_cancelled" },
    { type: "auth_failed", code: "sign_in_timed_out", message: "Sign-in timed out." },
  ];

  for (const update of updates) {
    boundary.handler?.({ payload: update } as Event<CodexAuthUpdate>);
  }

  expect(received).toEqual([
    {
      type: "auth_prompt",
      id: "method",
      prompt: {
        type: "select",
        message: "Choose a sign-in method",
        options: [
          { id: "browser", label: "Browser", description: "Use ChatGPT in your browser" },
          { id: "manual", label: "Manual code" },
        ],
      },
    },
    {
      type: "auth_prompt",
      id: "email",
      prompt: { type: "text", message: "Email" },
    },
    {
      type: "auth_prompt",
      id: "password",
      prompt: { type: "secret", message: "Password", placeholder: "Password" },
    },
    {
      type: "auth_prompt",
      id: "callback",
      prompt: { type: "manual_code", message: "Paste the callback code" },
    },
    { type: "auth_prompt_cancelled", id: "callback" },
    { type: "auth_complete" },
    { type: "auth_cancelled" },
    { type: "auth_failed", code: "sign_in_failed", message: "Sign-in timed out." },
  ]);
});

test("the auth session uses the exact native commands and pinned system plugins", async () => {
  boundary.openUrl.mockResolvedValue(undefined);
  boundary.writeText.mockResolvedValue(undefined);
  const session = await codexAuthAdapter.connect(vi.fn());

  await session.answer("manual-code", "private callback value");
  await session.cancel();
  await session.openExternal("https://auth.openai.com/device");
  await session.copyText("ABCD-EFGH");
  session.disconnect();
  session.disconnect();

  expect(boundary.invoke.mock.calls).toEqual([
    ["start_codex_sign_in"],
    ["answer_codex_auth_prompt", { promptId: "manual-code", value: "private callback value" }],
    ["cancel_codex_sign_in"],
  ]);
  expect(boundary.openUrl).toHaveBeenCalledWith("https://auth.openai.com/device");
  expect(boundary.writeText).toHaveBeenCalledWith("ABCD-EFGH");
  expect(boundary.unlisten).toHaveBeenCalledOnce();
});

test("external auth links reject malformed and non-HTTPS URLs before reaching macOS", async () => {
  const session = await codexAuthAdapter.connect(vi.fn());

  await expect(session.openExternal("http://auth.openai.com/device")).rejects.toThrow(
    "secure web links",
  );
  await expect(session.openExternal("file:///tmp/private-code")).rejects.toThrow(
    "secure web links",
  );
  await expect(session.openExternal("not a URL with private-code")).rejects.toThrow(
    "secure web links",
  );

  expect(boundary.openUrl).not.toHaveBeenCalled();
});

test("a failed native sign-in start releases its event listener", async () => {
  boundary.invoke.mockRejectedValueOnce({
    code: "signInUnavailable",
    message: "Sign-in is unavailable. Try again.",
  });

  await expect(codexAuthAdapter.connect(vi.fn())).rejects.toMatchObject({
    code: "signInUnavailable",
  });

  expect(boundary.order).toEqual(["listen"]);
  expect(boundary.unlisten).toHaveBeenCalledOnce();
});

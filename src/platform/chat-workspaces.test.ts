import { beforeEach, expect, test, vi } from "vitest";

import type { ChatSetupChangedEvent } from "../generated/ChatSetupChangedEvent";

import {
  cancelChatSetup,
  chatWorkspaceErrorMessage,
  createChat,
  listenToChatSetup,
  openChatTerminal,
  retryChatSetup,
} from "./chat-workspaces";

const tauri = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: tauri.listen }));

beforeEach(() => {
  tauri.invoke.mockReset();
  tauri.listen.mockReset();
});

test("chat workspace actions cross the typed host commands without paths or options", async () => {
  tauri.invoke.mockResolvedValue(undefined);

  await createChat(7, "Repair parser ownership");
  await retryChatSetup("chat-7");
  await cancelChatSetup("chat-7");
  await openChatTerminal("chat-7");

  expect(tauri.invoke.mock.calls).toEqual([
    ["create_chat", { request: { projectId: 7, prompt: "Repair parser ownership" } }],
    ["retry_chat_setup", { request: { chatId: "chat-7" } }],
    ["cancel_chat_setup", { request: { chatId: "chat-7" } }],
    ["open_chat_terminal", { request: { chatId: "chat-7" } }],
  ]);
});

test("setup changes subscribe to the narrow per-chat event", async () => {
  const onChange = vi.fn();
  tauri.listen.mockResolvedValue(vi.fn());

  await listenToChatSetup(onChange);
  const handler = tauri.listen.mock.calls[0][1] as (event: {
    payload: ChatSetupChangedEvent;
  }) => void;
  const event: ChatSetupChangedEvent = {
    chatId: "chat-7",
    setup: {
      phase: "running",
      failure: null,
      exitCode: null,
      signal: null,
      attempt: 1,
      log: "",
    },
  };
  handler({ payload: event });

  expect(tauri.listen).toHaveBeenCalledWith("chat-workspace://setup-changed", expect.any(Function));
  expect(onChange).toHaveBeenCalledWith(event);
});

test("host failures keep actionable product copy", () => {
  expect(
    chatWorkspaceErrorMessage(
      {
        code: "freshMainUnavailable",
        message: "Più couldn’t fetch a fresh origin/main. Check remote access and try again.",
      },
      "fallback",
    ),
  ).toMatch(/fresh origin\/main/);
  expect(chatWorkspaceErrorMessage(new Error("transport"), "fallback")).toBe("fallback");
});

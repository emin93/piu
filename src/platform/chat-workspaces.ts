import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ChatSetupChangedEvent } from "../generated/ChatSetupChangedEvent";
import type { ChatSetupSummary } from "../generated/ChatSetupSummary";
import type { ChatTerminalRequest } from "../generated/ChatTerminalRequest";
import type { ChatWorkspaceCommandError } from "../generated/ChatWorkspaceCommandError";
import type { ChatWorkspaceCommandErrorCode } from "../generated/ChatWorkspaceCommandErrorCode";
import type { CreateChatResponse } from "../generated/CreateChatResponse";
import type { PromptAttachment } from "../generated/PromptAttachment";

export type { ChatSetupChangedEvent } from "../generated/ChatSetupChangedEvent";
export type { ChatSetupSummary } from "../generated/ChatSetupSummary";

const CHAT_SETUP_CHANGED_EVENT = "chat-workspace://setup-changed";
const CHAT_WORKSPACE_ERROR_CODES = new Set<ChatWorkspaceCommandErrorCode>([
  "emptyPrompt",
  "projectNotFound",
  "chatNotFound",
  "freshMainUnavailable",
  "setupAlreadyRunning",
  "creationFailed",
  "storageUnavailable",
  "invalidAttachment",
]);

export function createChat(
  projectId: number,
  prompt: string,
  attachments: readonly PromptAttachment[],
) {
  return invoke<CreateChatResponse>("create_chat", {
    request: { attachments, projectId, prompt },
  });
}

export function retryChatSetup(chatId: string) {
  return invoke<ChatSetupSummary>("retry_chat_setup", { request: { chatId } });
}

export function cancelChatSetup(chatId: string) {
  return invoke<void>("cancel_chat_setup", { request: { chatId } });
}

export function openChatTerminal(chatId: string) {
  return invoke<ChatTerminalRequest>("open_chat_terminal", { request: { chatId } });
}

export function listenToChatSetup(onChange: (event: ChatSetupChangedEvent) => void) {
  return listen<ChatSetupChangedEvent>(CHAT_SETUP_CHANGED_EVENT, ({ payload }) => {
    onChange(payload);
  });
}

export function chatWorkspaceErrorMessage(error: unknown, fallback: string) {
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    CHAT_WORKSPACE_ERROR_CODES.has((error as ChatWorkspaceCommandError).code) &&
    "message" in error &&
    typeof (error as ChatWorkspaceCommandError).message === "string"
  ) {
    return (error as ChatWorkspaceCommandError).message;
  }
  return fallback;
}

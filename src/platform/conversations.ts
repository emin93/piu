import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ChatRuntimeChangedEvent } from "../generated/ChatRuntimeChangedEvent";
import type { ChatRuntimeCommandError } from "../generated/ChatRuntimeCommandError";
import type { ChatRuntimeCommandErrorCode } from "../generated/ChatRuntimeCommandErrorCode";
import type { ConversationEvent as NativeConversationEvent } from "../generated/ConversationEvent";
import type { ConversationItem as NativeConversationItem } from "../generated/ConversationItem";
import type { ConversationPromptRequest as NativeConversationPromptRequest } from "../generated/ConversationPromptRequest";
import type { ConversationSnapshot as NativeConversationSnapshot } from "../generated/ConversationSnapshot";
import type { OpenChatRuntimeRequest } from "../generated/OpenChatRuntimeRequest";

const CHAT_RUNTIME_CHANGED_EVENT = "chat-runtime://changed";
const CHAT_RUNTIME_ERROR_CODES = new Set<ChatRuntimeCommandErrorCode>([
  "emptyMessage",
  "chatNotFound",
  "setupIncomplete",
  "notActive",
  "runtimeUnavailable",
  "authenticationRequired",
  "conversationFailed",
  "storageUnavailable",
]);

export interface ConversationMessage {
  id: string;
  kind: "message";
  role: "user" | "assistant";
  text: string;
}

export type ConversationToolStatus = "running" | "succeeded" | "failed";

export interface ConversationTool {
  detail: string;
  id: string;
  kind: "tool";
  name: string;
  status: ConversationToolStatus;
}

export interface ConversationReasoning {
  id: string;
  kind: "reasoning";
  text: string;
}

export interface ConversationUsage {
  cacheReadTokens: number | null;
  id: string;
  inputTokens: number;
  kind: "usage";
  outputTokens: number;
}

export type ConversationItem =
  ConversationMessage | ConversationReasoning | ConversationTool | ConversationUsage;

export type ConversationPhase = "idle" | "running" | "stopped" | "failed";

export interface ConversationSnapshot {
  failure: string | null;
  items: readonly ConversationItem[];
  phase: ConversationPhase;
}

export interface ConversationTextDelta {
  delta: string;
  itemId: string;
  type: "text-delta";
}

export interface ConversationItemAdded {
  item: ConversationItem;
  type: "item-added";
}

export interface ConversationReasoningDelta {
  delta: string;
  itemId: string;
  type: "reasoning-delta";
}

export interface ConversationToolUpdate {
  detail: string;
  itemId: string;
  status: ConversationToolStatus;
  type: "tool-update";
}

export interface ConversationTurnCompleted {
  type: "turn-completed";
}

export interface ConversationTurnFailed {
  message: string;
  type: "turn-failed";
}

export interface ConversationTurnStarted {
  type: "turn-started";
}

export interface ConversationTurnStopped {
  type: "turn-stopped";
}

export interface ConversationUsageUpdate {
  cacheReadTokens: number | null;
  inputTokens: number;
  itemId: string;
  outputTokens: number;
  type: "usage-update";
}

export type ConversationEvent =
  | ConversationItemAdded
  | ConversationReasoningDelta
  | ConversationTextDelta
  | ConversationToolUpdate
  | ConversationTurnCompleted
  | ConversationTurnFailed
  | ConversationTurnStarted
  | ConversationTurnStopped
  | ConversationUsageUpdate;

export interface ConversationConnection {
  disconnect: () => void;
  snapshot: ConversationSnapshot;
}

export interface ConversationPromptRequest {
  chatId: string;
  streamingBehavior: "steer";
  text: string;
}

export interface ConversationAdapter {
  connect: (
    chatId: string,
    onEvent: (event: ConversationEvent) => void,
  ) => Promise<ConversationConnection>;
  prompt: (request: ConversationPromptRequest) => Promise<void>;
  stop: (chatId: string) => Promise<void>;
}

export function conversationErrorMessage(error: unknown, fallback: string) {
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    CHAT_RUNTIME_ERROR_CODES.has((error as ChatRuntimeCommandError).code) &&
    "message" in error &&
    typeof (error as ChatRuntimeCommandError).message === "string"
  ) {
    return (error as ChatRuntimeCommandError).message;
  }
  return fallback;
}

export function conversationRequiresCodexSignIn(error: unknown) {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === "authenticationRequired"
  );
}

function mapNativeItem(item: NativeConversationItem): ConversationItem {
  switch (item.kind) {
    case "message":
      return {
        id: item.id,
        kind: "message",
        role: item.role,
        text: item.text,
      };
    case "reasoning":
      return { id: item.id, kind: "reasoning", text: item.text };
    case "tool":
      return {
        detail: item.detail,
        id: item.id,
        kind: "tool",
        name: item.name,
        status: item.status,
      };
    case "usage":
      return {
        cacheReadTokens: item.cacheReadTokens,
        id: item.id,
        inputTokens: item.inputTokens,
        kind: "usage",
        outputTokens: item.outputTokens,
      };
  }
}

function mapNativeEvent(event: NativeConversationEvent): ConversationEvent {
  switch (event.type) {
    case "item-added":
      return { item: mapNativeItem(event.item), type: "item-added" };
    case "text-delta":
      return { delta: event.delta, itemId: event.itemId, type: "text-delta" };
    case "reasoning-delta":
      return { delta: event.delta, itemId: event.itemId, type: "reasoning-delta" };
    case "tool-update":
      return {
        detail: event.detail,
        itemId: event.itemId,
        status: event.status,
        type: "tool-update",
      };
    case "usage-update":
      return {
        cacheReadTokens: event.cacheReadTokens,
        inputTokens: event.inputTokens,
        itemId: event.itemId,
        outputTokens: event.outputTokens,
        type: "usage-update",
      };
    case "turn-started":
      return { type: "turn-started" };
    case "turn-completed":
      return { type: "turn-completed" };
    case "turn-stopped":
      return { type: "turn-stopped" };
    case "turn-failed":
      return { message: event.message, type: "turn-failed" };
  }
}

function mapNativeSnapshot(snapshot: NativeConversationSnapshot): ConversationSnapshot {
  return {
    failure: snapshot.failure,
    items: snapshot.items.map(mapNativeItem),
    phase: snapshot.phase,
  };
}

function releaseListener(unlisten: () => void) {
  try {
    unlisten();
  } catch {
    return;
  }
}

export const tauriConversationAdapter: ConversationAdapter = {
  async connect(chatId, onEvent) {
    const request: OpenChatRuntimeRequest = { chatId };
    const unlisten = await listen<ChatRuntimeChangedEvent>(
      CHAT_RUNTIME_CHANGED_EVENT,
      ({ payload }) => {
        if (payload.chatId === chatId) onEvent(mapNativeEvent(payload.event));
      },
    );
    let snapshot: NativeConversationSnapshot;
    try {
      snapshot = await invoke<NativeConversationSnapshot>("open_chat_runtime", {
        request,
      });
    } catch (error) {
      releaseListener(unlisten);
      throw error;
    }
    let disconnected = false;

    return {
      disconnect() {
        if (disconnected) return;
        disconnected = true;
        releaseListener(unlisten);
      },
      snapshot: mapNativeSnapshot(snapshot),
    };
  },
  prompt(request) {
    const nativeRequest: NativeConversationPromptRequest = request;
    return invoke<void>("send_chat_message", { request: nativeRequest });
  },
  stop(chatId) {
    const request: OpenChatRuntimeRequest = { chatId };
    return invoke<void>("abort_chat_turn", { request });
  },
};

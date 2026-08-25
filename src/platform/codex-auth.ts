import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { CodexAuthUpdate as NativeCodexAuthUpdate } from "../generated/CodexAuthUpdate";

const CODEX_AUTH_UPDATE_EVENT = "codex-auth://update";

export interface CodexAuthLink {
  label?: string;
  url: string;
}

export type CodexAuthNotification =
  | { links?: readonly CodexAuthLink[]; message: string; type: "info" }
  | { instructions?: string; type: "auth_url"; url: string }
  | {
      expiresInSeconds?: number;
      intervalSeconds?: number;
      type: "device_code";
      userCode: string;
      verificationUri: string;
    }
  | { message: string; type: "progress" };

export interface CodexAuthOption {
  description?: string;
  id: string;
  label: string;
}

export type CodexAuthPrompt =
  | { message: string; options: readonly CodexAuthOption[]; type: "select" }
  | { message: string; placeholder?: string; type: "text" | "secret" | "manual_code" };

export type CodexAuthRecord =
  | { event: CodexAuthNotification; type: "auth_event" }
  | { id: string; prompt: CodexAuthPrompt; type: "auth_prompt" }
  | { id: string; type: "auth_prompt_cancelled" }
  | { type: "auth_complete" }
  | { type: "auth_cancelled" }
  | { code: "sign_in_failed"; message: string; type: "auth_failed" };

export interface CodexAuthSession {
  answer: (promptId: string, value: string) => Promise<void>;
  cancel: () => Promise<void>;
  copyText: (text: string) => Promise<void>;
  disconnect: () => void;
  openExternal: (url: string) => Promise<void>;
}

export interface CodexAuthAdapter {
  connect: (receive: (record: CodexAuthRecord) => void) => Promise<CodexAuthSession>;
}

type NativeNotification = Extract<NativeCodexAuthUpdate, { type: "auth_event" }>["event"];
type NativePrompt = Extract<NativeCodexAuthUpdate, { type: "auth_prompt" }>["prompt"];

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isNullableString(value: unknown): value is string | null {
  return typeof value === "string" || value === null;
}

function isNullableNumber(value: unknown): value is number | null {
  return (typeof value === "number" && Number.isFinite(value)) || value === null;
}

function isNativeLink(value: unknown): boolean {
  return isObject(value) && typeof value.url === "string" && isNullableString(value.label);
}

function isNativeOption(value: unknown): boolean {
  return (
    isObject(value) &&
    typeof value.id === "string" &&
    typeof value.label === "string" &&
    isNullableString(value.description)
  );
}

function isNativeNotification(value: unknown): boolean {
  if (!isObject(value) || typeof value.type !== "string") return false;
  switch (value.type) {
    case "info":
      return (
        typeof value.message === "string" &&
        Array.isArray(value.links) &&
        value.links.every(isNativeLink)
      );
    case "auth_url":
      return typeof value.url === "string" && isNullableString(value.instructions);
    case "device_code":
      return (
        typeof value.userCode === "string" &&
        typeof value.verificationUri === "string" &&
        isNullableNumber(value.intervalSeconds) &&
        isNullableNumber(value.expiresInSeconds)
      );
    case "progress":
      return typeof value.message === "string";
    default:
      return false;
  }
}

function isNativePrompt(value: unknown): boolean {
  if (!isObject(value) || typeof value.type !== "string" || typeof value.message !== "string") {
    return false;
  }
  switch (value.type) {
    case "select":
      return Array.isArray(value.options) && value.options.every(isNativeOption);
    case "text":
    case "secret":
    case "manual_code":
      return isNullableString(value.placeholder);
    default:
      return false;
  }
}

function isNativeUpdate(value: unknown): value is NativeCodexAuthUpdate {
  if (!isObject(value) || typeof value.type !== "string") return false;
  switch (value.type) {
    case "auth_event":
      return isNativeNotification(value.event);
    case "auth_prompt":
      return typeof value.id === "string" && isNativePrompt(value.prompt);
    case "auth_prompt_cancelled":
      return typeof value.id === "string";
    case "auth_complete":
    case "auth_cancelled":
      return true;
    case "auth_failed":
      return typeof value.code === "string" && typeof value.message === "string";
    default:
      return false;
  }
}

function mapNativeNotification(event: NativeNotification): CodexAuthNotification {
  switch (event.type) {
    case "info":
      return {
        type: "info",
        message: event.message,
        links: event.links.map(({ label, url }) => ({
          ...(label === null ? {} : { label }),
          url,
        })),
      };
    case "auth_url":
      return {
        type: "auth_url",
        url: event.url,
        ...(event.instructions === null ? {} : { instructions: event.instructions }),
      };
    case "device_code":
      return {
        type: "device_code",
        userCode: event.userCode,
        verificationUri: event.verificationUri,
        ...(event.intervalSeconds === null ? {} : { intervalSeconds: event.intervalSeconds }),
        ...(event.expiresInSeconds === null ? {} : { expiresInSeconds: event.expiresInSeconds }),
      };
    case "progress":
      return { type: "progress", message: event.message };
  }
}

function mapNativePrompt(prompt: NativePrompt): CodexAuthPrompt {
  switch (prompt.type) {
    case "select":
      return {
        type: "select",
        message: prompt.message,
        options: prompt.options.map(({ description, id, label }) => ({
          id,
          label,
          ...(description === null ? {} : { description }),
        })),
      };
    case "text":
    case "secret":
    case "manual_code":
      return {
        type: prompt.type,
        message: prompt.message,
        ...(prompt.placeholder === null ? {} : { placeholder: prompt.placeholder }),
      };
  }
}

function mapNativeUpdate(value: unknown): CodexAuthRecord | undefined {
  if (!isNativeUpdate(value)) return undefined;
  const update = value;
  switch (update.type) {
    case "auth_event":
      return { type: "auth_event", event: mapNativeNotification(update.event) };
    case "auth_prompt":
      return { type: "auth_prompt", id: update.id, prompt: mapNativePrompt(update.prompt) };
    case "auth_prompt_cancelled":
      return { type: "auth_prompt_cancelled", id: update.id };
    case "auth_complete":
      return { type: "auth_complete" };
    case "auth_cancelled":
      return { type: "auth_cancelled" };
    case "auth_failed":
      return { type: "auth_failed", code: "sign_in_failed", message: update.message };
  }
}

async function openSecureUrl(url: string): Promise<void> {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:" || parsed.hostname.length === 0) throw new Error();
  } catch {
    throw new Error("Più can only open secure web links.");
  }
  await openUrl(url);
}

export const codexAuthAdapter: CodexAuthAdapter = {
  async connect(receive) {
    const unlisten = await listen<unknown>(CODEX_AUTH_UPDATE_EVENT, ({ payload }) => {
      const record = mapNativeUpdate(payload);
      if (record) receive(record);
    });
    let connected = true;
    const disconnect = () => {
      if (!connected) return;
      connected = false;
      unlisten();
    };
    try {
      await invoke("start_codex_sign_in");
    } catch (error) {
      disconnect();
      throw error;
    }

    return {
      answer: (promptId, value) =>
        invoke("answer_codex_auth_prompt", {
          promptId,
          value,
        }),
      cancel: () => invoke("cancel_codex_sign_in"),
      copyText: (text) => writeText(text),
      disconnect,
      openExternal: openSecureUrl,
    };
  },
};

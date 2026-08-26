import type { ConversationAdapter } from "@/platform/conversations";
import type { ModelControlsAdapter } from "@/platform/model-controls";
import type { PromptAttachment } from "@/platform/prompt-attachments";

import { ModelControlsController } from "../model-controls/model-controls-controller";
import { ConversationController } from "./conversation-controller";

const MAX_CACHED_CHAT_SESSIONS = 3;

export interface ChatConversationSession {
  composer: ChatComposerSession;
  controller: ConversationController;
  modelControls: ModelControlsController;
}

const cachedChatSessions = new WeakMap<object, Map<string, ChatConversationSession>>();

class ChatComposerSession {
  #attachments: PromptAttachment[] = [];
  #draft = "";

  get attachments() {
    return this.#attachments;
  }

  get draft() {
    return this.#draft;
  }

  rememberAttachments(attachments: PromptAttachment[]) {
    this.#attachments = attachments;
  }

  rememberDraft(draft: string) {
    this.#draft = draft;
  }
}

export function createChatConversationSession(
  adapter: ConversationAdapter,
  chatId: string,
  modelControlsAdapter: ModelControlsAdapter,
): ChatConversationSession {
  return {
    composer: new ChatComposerSession(),
    controller: new ConversationController(chatId, adapter),
    modelControls: new ModelControlsController(chatId, modelControlsAdapter),
  };
}

export function readCachedChatConversationSession(owner: object, chatId: string) {
  return cachedChatSessions.get(owner)?.get(chatId);
}

export function rememberCachedChatConversationSession(
  owner: object,
  chatId: string,
  session: ChatConversationSession,
) {
  let sessions = cachedChatSessions.get(owner);
  if (!sessions) {
    sessions = new Map();
    cachedChatSessions.set(owner, sessions);
  }

  const existing = sessions.get(chatId);
  if (existing !== session) {
    existing?.controller.dispose();
    existing?.modelControls.dispose();
  }
  sessions.delete(chatId);
  sessions.set(chatId, session);
  if (sessions.size > MAX_CACHED_CHAT_SESSIONS) {
    const oldestChatId = sessions.keys().next().value;
    if (oldestChatId) {
      const oldestSession = sessions.get(oldestChatId);
      sessions.delete(oldestChatId);
      oldestSession?.controller.dispose();
      oldestSession?.modelControls.dispose();
    }
  }
}

export function invalidateCachedChatConversationSession(owner: object, chatId: string) {
  const sessions = cachedChatSessions.get(owner);
  const session = sessions?.get(chatId);
  if (!sessions || !session) return;
  sessions.delete(chatId);
  session.controller.dispose();
  session.modelControls.dispose();
}

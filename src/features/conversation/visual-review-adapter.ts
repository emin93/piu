import type { ConversationAdapter, ConversationSnapshot } from "@/platform/conversations";

const VISUAL_REVIEW_SNAPSHOT: ConversationSnapshot = {
  failure: null,
  inputRequest: null,
  items: [
    {
      id: "review-user",
      kind: "message",
      queued: false,
      role: "user",
      text: "Verify the packaged runtime before release.",
    },
    {
      id: "review-reasoning",
      kind: "reasoning",
      text: "Inspecting the bundled runtime and repository state.",
    },
    {
      id: "review-assistant",
      kind: "message",
      queued: false,
      role: "assistant",
      text: "The packaged runtime is streaming from the stored Pi session.",
    },
    {
      detail: "Read package.json and the pinned runtime manifest.",
      id: "review-tool-succeeded",
      kind: "tool",
      name: "Read package manifest",
      status: "succeeded",
    },
    {
      detail: "Running TypeScript, Rust, and runtime contract checks",
      id: "review-tool-running",
      kind: "tool",
      name: "Run release checks",
      status: "running",
    },
    {
      detail: "Safari remote automation is unavailable",
      id: "review-tool-failed",
      kind: "tool",
      name: "Inspect optional trace",
      status: "failed",
    },
    {
      cacheReadTokens: 800,
      id: "review-usage",
      inputTokens: 1_200,
      kind: "usage",
      outputTokens: 84,
    },
  ],
  phase: "running",
};

export const visualReviewConnectionRecoveryAdapter: ConversationAdapter = {
  answerInput: () => Promise.resolve(),
  connect: () =>
    Promise.reject(
      Object.assign(new Error("Più couldn’t save this conversation. Try again."), {
        code: "storageUnavailable",
      }),
    ),
  prompt: () => Promise.resolve(),
  stop: () => Promise.resolve(),
};

export const visualReviewConversationAdapter: ConversationAdapter = {
  answerInput: () => Promise.resolve(),
  connect: () =>
    Promise.resolve({
      disconnect: () => undefined,
      snapshot: VISUAL_REVIEW_SNAPSHOT,
    }),
  prompt: () => Promise.resolve(),
  stop: () => Promise.resolve(),
};

export const visualReviewSendRecoveryAdapter: ConversationAdapter = {
  answerInput: () => Promise.resolve(),
  connect: () =>
    Promise.resolve({
      disconnect: () => undefined,
      snapshot: {
        failure: null,
        inputRequest: null,
        items: [
          {
            id: "recovery-assistant",
            kind: "message",
            queued: false,
            role: "assistant",
            text: "Your earlier conversation is safe and ready to continue.",
          },
        ],
        phase: "stopped",
      },
    }),
  prompt: () =>
    Promise.reject(
      Object.assign(new Error("Sign in to Codex to continue this conversation."), {
        code: "authenticationRequired",
      }),
    ),
  stop: () => Promise.resolve(),
};

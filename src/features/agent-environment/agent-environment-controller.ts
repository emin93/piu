import type { AgentEnvironmentSnapshot } from "@/generated/AgentEnvironmentSnapshot";
import type { AgentResourceId } from "@/generated/AgentResourceId";
import type { AgentResourcePreferenceChange } from "@/generated/AgentResourcePreferenceChange";
import type { AgentResourcePreferenceScope } from "@/generated/AgentResourcePreferenceScope";
import type { AgentEnvironmentAdapter } from "@/platform/agent-environment";

type Listener = () => void;

type RetryAttempt =
  | { kind: "load" }
  | {
      enabled: boolean;
      kind: "resource";
      resource: AgentResourceId;
      scope: AgentResourcePreferenceScope;
    };

export interface AgentEnvironmentStoreSnapshot {
  environment: AgentEnvironmentSnapshot | null;
  error: string | null;
  pendingResource: AgentResourceId | null;
  phase: "changing" | "failed" | "loading" | "ready";
  status: string | null;
}

function errorMessage(error: unknown, fallback: string) {
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string" && message.trim()) return message;
  }
  return fallback;
}

function changeStatus(change: AgentResourcePreferenceChange) {
  const notices: string[] = [];
  if (change.restartFailedChatCount > 0) {
    const chats = change.restartFailedChatCount === 1 ? "chat" : "chats";
    notices.push(
      `The change was saved, but Più couldn’t restart ${change.restartFailedChatCount} idle ${chats}. Open ${change.restartFailedChatCount === 1 ? "it" : "them"} to reconnect.`,
    );
  }
  if (change.deferredChatCount > 0) {
    const chats = change.deferredChatCount === 1 ? "chat" : "chats";
    notices.push(
      `${change.deferredChatCount} active ${chats} will use this change after the current step.`,
    );
  }
  return notices.length > 0 ? notices.join(" ") : null;
}

export class AgentEnvironmentController {
  readonly #adapter: AgentEnvironmentAdapter;
  #generation = 0;
  readonly #listeners = new Set<Listener>();
  readonly #projectId: number | null;
  #retry: RetryAttempt = { kind: "load" };
  #snapshot: AgentEnvironmentStoreSnapshot;

  constructor(projectId: number | null, adapter: AgentEnvironmentAdapter) {
    this.#adapter = adapter;
    this.#projectId = projectId;
    this.#snapshot =
      projectId !== null
        ? {
            environment: null,
            error: null,
            pendingResource: null,
            phase: "loading",
            status: null,
          }
        : {
            environment: null,
            error: "Open a repository to inspect its models and resources.",
            pendingResource: null,
            phase: "failed",
            status: null,
          };
  }

  getSnapshot = () => this.#snapshot;

  subscribe = (listener: Listener) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  #publish(snapshot: AgentEnvironmentStoreSnapshot) {
    this.#snapshot = snapshot;
    for (const listener of this.#listeners) listener();
  }

  load = async () => {
    if (this.#projectId === null || this.#snapshot.phase === "changing") return;
    const generation = ++this.#generation;
    this.#retry = { kind: "load" };
    this.#publish({
      environment: this.#snapshot.environment,
      error: null,
      pendingResource: null,
      phase: "loading",
      status: null,
    });
    try {
      const environment = await this.#adapter.get(this.#projectId);
      if (generation !== this.#generation) return;
      this.#publish({
        environment,
        error: null,
        pendingResource: null,
        phase: "ready",
        status: null,
      });
    } catch (cause) {
      if (generation !== this.#generation) return;
      this.#publish({
        environment: this.#snapshot.environment,
        error: errorMessage(
          cause,
          "Più couldn’t inspect this project’s agent environment. Try again.",
        ),
        pendingResource: null,
        phase: "failed",
        status: null,
      });
    }
  };

  setResourceEnabled = async (
    resource: AgentResourceId,
    enabled: boolean,
    scope: AgentResourcePreferenceScope,
  ) => {
    if (this.#projectId === null || this.#snapshot.phase === "changing") return;
    const generation = ++this.#generation;
    this.#retry = { enabled, kind: "resource", resource, scope };
    const previous = this.#snapshot.environment;
    this.#publish({
      environment: previous,
      error: null,
      pendingResource: resource,
      phase: "changing",
      status: null,
    });
    let change: AgentResourcePreferenceChange;
    try {
      change = await this.#adapter.setEnabled(this.#projectId, scope, resource, enabled);
    } catch (cause) {
      if (generation !== this.#generation) return;
      this.#publish({
        environment: previous,
        error: errorMessage(cause, "Più couldn’t finish applying that resource change. Try again."),
        pendingResource: null,
        phase: "failed",
        status: null,
      });
      return;
    }
    if (generation !== this.#generation) return;

    const status = changeStatus(change);
    this.#retry = { kind: "load" };
    try {
      const environment = await this.#adapter.get(this.#projectId);
      if (generation !== this.#generation) return;
      this.#publish({ environment, error: null, pendingResource: null, phase: "ready", status });
    } catch {
      if (generation !== this.#generation) return;
      this.#publish({
        environment: null,
        error:
          "The change was saved, but Più couldn’t refresh models and resources. Retry to load the saved state.",
        pendingResource: null,
        phase: "failed",
        status,
      });
    }
  };

  retry = async () => {
    const attempt = this.#retry;
    if (attempt.kind === "load") {
      await this.load();
      return;
    }
    await this.setResourceEnabled(attempt.resource, attempt.enabled, attempt.scope);
  };

  dispose() {
    this.#generation += 1;
  }
}

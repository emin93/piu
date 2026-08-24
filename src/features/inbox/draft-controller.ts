import type { InboxSnapshot } from "@/platform/project-inbox";

export type DraftPersistenceStatus =
  { state: "idle" | "saving" | "saved" } | { state: "failed"; message: string };

export interface ProjectDraftState {
  prompt: string;
  status: DraftPersistenceStatus;
}

type PersistDraft = (projectId: number, prompt: string) => Promise<unknown>;
type DraftListener = () => void;

interface DraftEntry extends ProjectDraftState {
  local: boolean;
}

const EMPTY_DRAFT: ProjectDraftState = { prompt: "", status: { state: "idle" } };
const SAVE_DELAY_MS = 250;

export class ProjectDraftController {
  readonly #allListeners = new Set<DraftListener>();
  readonly #entries = new Map<number, DraftEntry>();
  readonly #generations = new Map<number, number>();
  readonly #listeners = new Map<number, Set<DraftListener>>();
  readonly #persist: PersistDraft;
  readonly #queues = new Map<number, Promise<void>>();
  readonly #saveDelayMs: number;
  readonly #timers = new Map<number, ReturnType<typeof setTimeout>>();
  readonly #toFailureMessage: (error: unknown) => string;
  #revision = 0;

  constructor(
    persist: PersistDraft,
    options: {
      saveDelayMs?: number;
      toFailureMessage?: (error: unknown) => string;
    } = {},
  ) {
    this.#persist = persist;
    this.#saveDelayMs = options.saveDelayMs ?? SAVE_DELAY_MS;
    this.#toFailureMessage =
      options.toFailureMessage ?? (() => "Couldn't save this draft. Keep Più open and try again.");
  }

  get(projectId: number): ProjectDraftState {
    return this.#entries.get(projectId) ?? EMPTY_DRAFT;
  }

  subscribe(projectId: number, listener: DraftListener) {
    const listeners = this.#listeners.get(projectId) ?? new Set<DraftListener>();
    listeners.add(listener);
    this.#listeners.set(projectId, listeners);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) this.#listeners.delete(projectId);
    };
  }

  getRevision = () => this.#revision;

  subscribeAll = (listener: DraftListener) => {
    this.#allListeners.add(listener);
    return () => this.#allListeners.delete(listener);
  };

  reconcile(snapshot: InboxSnapshot) {
    const incomingByProject = new Map(
      snapshot.drafts.map((draft) => [draft.projectId, draft.prompt] as const),
    );

    for (const project of snapshot.projects) {
      const prompt = incomingByProject.get(project.id) ?? "";
      const current = this.#entries.get(project.id);
      if (current?.local && current.prompt !== prompt) continue;
      const status: DraftPersistenceStatus = prompt ? { state: "saved" } : { state: "idle" };
      if (
        current &&
        current.prompt === prompt &&
        current.status.state === status.state &&
        !current.local
      ) {
        continue;
      }
      this.#entries.set(project.id, { prompt, status, local: false });
      this.#notify(project.id);
    }
  }

  overlay(snapshot: InboxSnapshot): InboxSnapshot {
    const projectIds = new Set(snapshot.projects.map((project) => project.id));
    const drafts = snapshot.drafts.filter((draft) => !this.#entries.has(draft.projectId));

    for (const [projectId, entry] of this.#entries) {
      if (!projectIds.has(projectId) || !entry.prompt) continue;
      drafts.push({ projectId, prompt: entry.prompt, updatedAtMs: Date.now() });
    }

    return { ...snapshot, drafts };
  }

  change(projectId: number, prompt: string) {
    const generation = (this.#generations.get(projectId) ?? 0) + 1;
    this.#generations.set(projectId, generation);
    this.#entries.set(projectId, { prompt, status: { state: "saving" }, local: true });
    this.#notify(projectId);

    const previousTimer = this.#timers.get(projectId);
    if (previousTimer) clearTimeout(previousTimer);
    this.#timers.set(
      projectId,
      setTimeout(() => void this.flush(projectId), this.#saveDelayMs),
    );
  }

  async retry(projectId: number) {
    const entry = this.#entries.get(projectId);
    if (!entry || entry.status.state !== "failed") return;
    this.change(projectId, entry.prompt);
    await this.flush(projectId);
  }

  async flush(projectId: number) {
    const timer = this.#timers.get(projectId);
    if (timer) clearTimeout(timer);
    this.#timers.delete(projectId);

    const entry = this.#entries.get(projectId);
    if (!entry || entry.status.state !== "saving") {
      await this.#queues.get(projectId);
      return;
    }

    const generation = this.#generations.get(projectId) ?? 0;
    const prompt = entry.prompt;
    const previous = this.#queues.get(projectId) ?? Promise.resolve();
    const queued = previous.then(async () => {
      try {
        await this.#persist(projectId, prompt);
        if (this.#generations.get(projectId) !== generation) return;
        this.#entries.set(projectId, { prompt, status: { state: "saved" }, local: true });
      } catch (error: unknown) {
        if (this.#generations.get(projectId) !== generation) return;
        this.#entries.set(projectId, {
          prompt,
          status: { state: "failed", message: this.#toFailureMessage(error) },
          local: true,
        });
      }
      this.#notify(projectId);
    });

    this.#queues.set(projectId, queued);
    await queued;
    if (this.#queues.get(projectId) === queued) this.#queues.delete(projectId);
  }

  async flushAll() {
    for (const projectId of [...this.#timers.keys()]) void this.flush(projectId);
    await Promise.all([...this.#queues.values()]);
    if ([...this.#entries.values()].some(({ status }) => status.state === "failed")) {
      throw new Error("One or more drafts could not be saved.");
    }
  }

  cancel(projectId: number) {
    const timer = this.#timers.get(projectId);
    if (timer) clearTimeout(timer);
    this.#timers.delete(projectId);
    this.#queues.delete(projectId);
    this.#generations.set(projectId, (this.#generations.get(projectId) ?? 0) + 1);
  }

  forget(projectId: number) {
    this.cancel(projectId);
    this.#entries.delete(projectId);
    this.#notify(projectId);
  }

  #notify(projectId: number) {
    this.#revision += 1;
    for (const listener of this.#listeners.get(projectId) ?? []) listener();
    for (const listener of this.#allListeners) listener();
  }
}

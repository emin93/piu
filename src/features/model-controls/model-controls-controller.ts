import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";
import type { ModelRouteId } from "@/generated/ModelRouteId";
import type { ReasoningEffort } from "@/generated/ReasoningEffort";
import type { ModelControlsAdapter } from "@/platform/model-controls";

type Listener = () => void;

export type PendingModelControl =
  { effort: ReasoningEffort; kind: "effort" } | { kind: "route"; route: ModelRouteId };

export interface ModelControlsStoreSnapshot {
  controls: ModelControlsSnapshot | null;
  error: string | null;
  pending: PendingModelControl | null;
  phase: "changing" | "failed" | "loading" | "ready";
}

const INITIAL_SNAPSHOT: ModelControlsStoreSnapshot = {
  controls: null,
  error: null,
  pending: null,
  phase: "loading",
};

const EFFORT_LABELS: Record<ReasoningEffort, string> = {
  off: "Off",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra High",
  max: "Maximum",
};

export function reasoningEffortLabel(effort: ReasoningEffort) {
  return EFFORT_LABELS[effort];
}

function sameRoute(left: ModelRouteId, right: ModelRouteId) {
  return left.modelId === right.modelId && left.provider === right.provider;
}

function selectedRouteName(controls: ModelControlsSnapshot) {
  return (
    controls.routes.find((route) => sameRoute(route.id, controls.selectedRoute))?.name ??
    controls.selectedRoute.modelId
  );
}

export class ModelControlsController {
  readonly #adapter: ModelControlsAdapter;
  readonly #chatId: string;
  #generation = 0;
  readonly #listeners = new Set<Listener>();
  #retry: (() => Promise<void>) | null = null;
  #snapshot = INITIAL_SNAPSHOT;

  constructor(chatId: string, adapter: ModelControlsAdapter) {
    this.#adapter = adapter;
    this.#chatId = chatId;
  }

  getSnapshot = () => this.#snapshot;

  subscribe = (listener: Listener) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  #publish(snapshot: ModelControlsStoreSnapshot) {
    this.#snapshot = snapshot;
    for (const listener of this.#listeners) listener();
  }

  load = async () => {
    const generation = ++this.#generation;
    const previousControls = this.#snapshot.controls;
    this.#retry = null;
    if (previousControls || this.#snapshot.error || this.#snapshot.phase !== "loading") {
      this.#publish({
        controls: previousControls,
        error: null,
        pending: null,
        phase: previousControls ? "changing" : "loading",
      });
    }
    try {
      const controls = await this.#adapter.get(this.#chatId);
      if (generation !== this.#generation) return;
      this.#publish({ controls, error: null, pending: null, phase: "ready" });
    } catch {
      if (generation !== this.#generation) return;
      this.#retry = this.load;
      this.#publish({
        controls: previousControls,
        error: previousControls
          ? "Couldn’t refresh model controls. Your current model is unchanged."
          : "Model controls are unavailable. Try again.",
        pending: null,
        phase: "failed",
      });
    }
  };

  selectRoute = async (route: ModelRouteId) => {
    const current = this.#snapshot.controls;
    if (!current || this.#snapshot.phase === "changing") return;
    if (sameRoute(current.selectedRoute, route)) return;
    const target = current.routes.find((candidate) => sameRoute(candidate.id, route));
    if (!target) {
      this.#retry = null;
      this.#publish({
        controls: current,
        error: "That model route is no longer available. Choose another model.",
        pending: null,
        phase: "failed",
      });
      return;
    }

    const generation = ++this.#generation;
    this.#retry = null;
    this.#publish({
      controls: current,
      error: null,
      pending: { kind: "route", route },
      phase: "changing",
    });
    try {
      const controls = await this.#adapter.selectRoute(this.#chatId, route);
      if (generation !== this.#generation) return;
      this.#publish({ controls, error: null, pending: null, phase: "ready" });
    } catch {
      if (generation !== this.#generation) return;
      this.#retry = () => this.selectRoute(route);
      this.#publish({
        controls: current,
        error: `Couldn’t switch to ${target.name}. Still using ${selectedRouteName(current)}.`,
        pending: null,
        phase: "failed",
      });
    }
  };

  selectEffort = async (effort: ReasoningEffort) => {
    const current = this.#snapshot.controls;
    if (!current || this.#snapshot.phase === "changing") return;
    if (current.selectedEffort === effort) return;
    if (!current.efforts.includes(effort)) {
      this.#retry = null;
      this.#publish({
        controls: current,
        error: `${reasoningEffortLabel(effort)} is not available for ${selectedRouteName(current)}.`,
        pending: null,
        phase: "failed",
      });
      return;
    }

    const generation = ++this.#generation;
    this.#retry = null;
    this.#publish({
      controls: current,
      error: null,
      pending: { effort, kind: "effort" },
      phase: "changing",
    });
    try {
      const controls = await this.#adapter.selectEffort(this.#chatId, effort);
      if (generation !== this.#generation) return;
      this.#publish({ controls, error: null, pending: null, phase: "ready" });
    } catch {
      if (generation !== this.#generation) return;
      this.#retry = () => this.selectEffort(effort);
      this.#publish({
        controls: current,
        error: `Couldn’t change reasoning effort. Still using ${reasoningEffortLabel(current.selectedEffort)}.`,
        pending: null,
        phase: "failed",
      });
    }
  };

  retry = async () => {
    await this.#retry?.();
  };

  markCurrentStepApplied = () => {
    const controls = this.#snapshot.controls;
    if (!controls?.appliesAfterCurrentStep) return;
    this.#publish({
      ...this.#snapshot,
      controls: { ...controls, appliesAfterCurrentStep: false },
    });
  };

  dispose() {
    this.#generation += 1;
    this.#retry = null;
  }
}

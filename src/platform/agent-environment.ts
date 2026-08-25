import { invoke } from "@tauri-apps/api/core";

import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";
import type { ProjectAgentEnvironmentRequest } from "@/generated/ProjectAgentEnvironmentRequest";
import type { SelectProjectModelRouteRequest } from "@/generated/SelectProjectModelRouteRequest";
import type { SelectProjectReasoningEffortRequest } from "@/generated/SelectProjectReasoningEffortRequest";

import type { ModelControlsAdapter } from "./model-controls";

export const tauriProjectModelControlsAdapter: ModelControlsAdapter<number> = {
  get(projectId) {
    const request: ProjectAgentEnvironmentRequest = { projectId };
    return invoke<ModelControlsSnapshot>("get_project_model_controls", { request });
  },
  selectEffort(projectId, effort) {
    const request: SelectProjectReasoningEffortRequest = { effort, projectId };
    return invoke<ModelControlsSnapshot>("select_project_reasoning_effort", { request });
  },
  selectRoute(projectId, route) {
    const request: SelectProjectModelRouteRequest = { projectId, route };
    return invoke<ModelControlsSnapshot>("select_project_model_route", { request });
  },
};

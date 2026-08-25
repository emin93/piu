import { invoke } from "@tauri-apps/api/core";

import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";
import type { AgentEnvironmentSnapshot } from "@/generated/AgentEnvironmentSnapshot";
import type { AgentResourceId } from "@/generated/AgentResourceId";
import type { AgentResourcePreferenceChange } from "@/generated/AgentResourcePreferenceChange";
import type { AgentResourcePreferenceScope } from "@/generated/AgentResourcePreferenceScope";
import type { ProjectAgentEnvironmentRequest } from "@/generated/ProjectAgentEnvironmentRequest";
import type { SelectProjectModelRouteRequest } from "@/generated/SelectProjectModelRouteRequest";
import type { SelectProjectReasoningEffortRequest } from "@/generated/SelectProjectReasoningEffortRequest";
import type { SetAgentResourceEnabledRequest } from "@/generated/SetAgentResourceEnabledRequest";

import type { ModelControlsAdapter } from "./model-controls";

export interface AgentEnvironmentAdapter {
  get: (projectId: number) => Promise<AgentEnvironmentSnapshot>;
  setEnabled: (
    projectId: number,
    scope: AgentResourcePreferenceScope,
    resource: AgentResourceId,
    enabled: boolean,
  ) => Promise<AgentResourcePreferenceChange>;
}

export const tauriAgentEnvironmentAdapter: AgentEnvironmentAdapter = {
  get(projectId) {
    const request: ProjectAgentEnvironmentRequest = { projectId };
    return invoke<AgentEnvironmentSnapshot>("get_project_agent_environment", { request });
  },
  setEnabled(projectId, scope, resource, enabled) {
    const request: SetAgentResourceEnabledRequest = { enabled, projectId, resource, scope };
    return invoke<AgentResourcePreferenceChange>("set_agent_resource_enabled", { request });
  },
};

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

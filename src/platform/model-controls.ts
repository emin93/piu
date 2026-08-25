import { invoke } from "@tauri-apps/api/core";

import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";
import type { ModelRouteId } from "@/generated/ModelRouteId";
import type { OpenChatRuntimeRequest } from "@/generated/OpenChatRuntimeRequest";
import type { ReasoningEffort } from "@/generated/ReasoningEffort";
import type { SelectModelRouteRequest } from "@/generated/SelectModelRouteRequest";
import type { SelectReasoningEffortRequest } from "@/generated/SelectReasoningEffortRequest";

export interface ModelControlsAdapter {
  get: (chatId: string) => Promise<ModelControlsSnapshot>;
  selectEffort: (chatId: string, effort: ReasoningEffort) => Promise<ModelControlsSnapshot>;
  selectRoute: (chatId: string, route: ModelRouteId) => Promise<ModelControlsSnapshot>;
}

export const tauriModelControlsAdapter: ModelControlsAdapter = {
  get(chatId) {
    const request: OpenChatRuntimeRequest = { chatId };
    return invoke<ModelControlsSnapshot>("get_model_controls", { request });
  },
  selectEffort(chatId, effort) {
    const request: SelectReasoningEffortRequest = { chatId, effort };
    return invoke<ModelControlsSnapshot>("select_reasoning_effort", { request });
  },
  selectRoute(chatId, route) {
    const request: SelectModelRouteRequest = { chatId, route };
    return invoke<ModelControlsSnapshot>("select_model_route", { request });
  },
};

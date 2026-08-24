import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ModelAssetStatus } from "../generated/ModelAssetStatus";
import type { ModelAssetCommandError } from "../generated/ModelAssetCommandError";

const MODEL_ASSET_STATUS_EVENT = "model-assets://status";

export class ModelAssetRequestError extends Error {
  readonly code: ModelAssetCommandError["code"];

  constructor(error: ModelAssetCommandError) {
    super(error.message);
    this.name = "ModelAssetRequestError";
    this.code = error.code;
  }
}

async function modelAssetInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (cause) {
    if (
      typeof cause === "object" &&
      cause !== null &&
      "code" in cause &&
      "message" in cause &&
      typeof cause.code === "string" &&
      typeof cause.message === "string"
    ) {
      throw new ModelAssetRequestError(cause as ModelAssetCommandError);
    }
    throw cause;
  }
}

export function getModelAssetStatus(): Promise<ModelAssetStatus> {
  return modelAssetInvoke<ModelAssetStatus>("model_asset_status");
}

export async function subscribeToModelAssetStatus(
  onStatus: (status: ModelAssetStatus) => void,
): Promise<() => void> {
  return listen<ModelAssetStatus>(MODEL_ASSET_STATUS_EVENT, ({ payload }) => onStatus(payload));
}

export function startModelDownload(): Promise<number> {
  return modelAssetInvoke<number>("start_model_download");
}

export function cancelModelDownload(): Promise<boolean> {
  return modelAssetInvoke<boolean>("cancel_model_download");
}

export function authorizeHuggingFace(token: string): Promise<void> {
  return modelAssetInvoke<void>("authorize_hugging_face", { token });
}

export function removeModelAssets(): Promise<ModelAssetStatus> {
  return modelAssetInvoke<ModelAssetStatus>("remove_model_assets");
}

export function retryModelAssetRecovery(): Promise<ModelAssetStatus> {
  return modelAssetInvoke<ModelAssetStatus>("retry_model_asset_recovery");
}

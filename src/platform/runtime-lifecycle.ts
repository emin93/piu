import { invoke } from "@tauri-apps/api/core";

export function hasActiveAgentTurn(): Promise<boolean> {
  return invoke<boolean>("has_active_agent_turn");
}

export function exitApplication(): Promise<void> {
  return invoke<void>("exit_application");
}

export function shutdownRuntimeProcesses(): Promise<void> {
  return invoke<void>("shutdown_runtime_processes");
}

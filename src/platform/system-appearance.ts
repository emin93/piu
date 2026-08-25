import { invoke } from "@tauri-apps/api/core";

export type SystemAppearance = "light" | "dark";

export async function readSystemAppearance(): Promise<SystemAppearance> {
  const appearance = await invoke<string>("system_appearance");
  if (appearance === "light" || appearance === "dark") return appearance;
  throw new Error("Più received an unsupported macOS appearance");
}

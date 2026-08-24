import { open } from "@tauri-apps/plugin-dialog";

export async function selectRepositoryDirectory(): Promise<string | null> {
  const selection = await open({
    directory: true,
    multiple: false,
    title: "Open Repository",
  });
  return typeof selection === "string" ? selection : null;
}

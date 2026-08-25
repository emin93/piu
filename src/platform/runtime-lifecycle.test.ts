import { beforeEach, expect, test, vi } from "vitest";

import { exitApplication, hasActiveAgentTurn, shutdownRuntimeProcesses } from "./runtime-lifecycle";

const tauri = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));

beforeEach(() => {
  tauri.invoke.mockReset();
  tauri.invoke.mockResolvedValue(undefined);
});

test("asks the native host to stop every owned runtime before the window closes", async () => {
  await shutdownRuntimeProcesses();

  expect(tauri.invoke).toHaveBeenCalledOnce();
  expect(tauri.invoke).toHaveBeenCalledWith("shutdown_runtime_processes");
});

test("asks the native host whether an agent turn is active", async () => {
  tauri.invoke.mockResolvedValue(true);

  await expect(hasActiveAgentTurn()).resolves.toBe(true);

  expect(tauri.invoke).toHaveBeenCalledWith("has_active_agent_turn");
});

test("asks the native host to terminate the application after shutdown", async () => {
  await exitApplication();

  expect(tauri.invoke).toHaveBeenCalledOnce();
  expect(tauri.invoke).toHaveBeenCalledWith("exit_application");
});

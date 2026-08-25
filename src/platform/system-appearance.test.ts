import { beforeEach, expect, test, vi } from "vitest";

import { readSystemAppearance } from "./system-appearance";

const boundary = vi.hoisted(() => ({ value: "light" }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(boundary.value)),
}));

beforeEach(() => {
  boundary.value = "light";
});

test("reads the effective macOS appearance through the native boundary", async () => {
  await expect(readSystemAppearance()).resolves.toBe("light");
  boundary.value = "dark";
  await expect(readSystemAppearance()).resolves.toBe("dark");
});

test("rejects an appearance outside Più's system-only theme contract", async () => {
  boundary.value = "automatic";
  await expect(readSystemAppearance()).rejects.toThrow("unsupported macOS appearance");
});

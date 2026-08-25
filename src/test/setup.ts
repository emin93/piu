import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

vi.mock("react-virtuoso", async (importOriginal) => {
  const original = await importOriginal<typeof import("react-virtuoso")>();
  const { MockVirtuoso } = await import("./mock-virtuoso");
  return { ...original, Virtuoso: MockVirtuoso };
});

afterEach(() => {
  cleanup();
  document.documentElement.removeAttribute("data-appearance");
});

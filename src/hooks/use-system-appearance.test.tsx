import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";

import { installMatchMedia } from "@/test/match-media";

import { useSystemAppearance } from "./use-system-appearance";

const nativeWindow = vi.hoisted(() => ({
  onFocusChanged: vi.fn(),
  onThemeChanged: vi.fn(),
}));
const nativeAppearance = vi.hoisted<{ value: "light" | "dark" }>(() => ({ value: "dark" }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => nativeWindow,
}));

vi.mock("@/platform/system-appearance", () => ({
  readSystemAppearance: vi.fn(() => Promise.resolve(nativeAppearance.value)),
}));

function AppearanceProbe() {
  const appearance = useSystemAppearance();
  return <div data-appearance={appearance} data-testid="appearance" />;
}

beforeEach(() => {
  nativeWindow.onFocusChanged.mockReset();
  nativeWindow.onThemeChanged.mockReset();
  nativeAppearance.value = "dark";
});

test("native macOS appearance wins at launch and follows later theme events", async () => {
  installMatchMedia("light");
  const unlistenFocus = vi.fn();
  const unlistenTheme = vi.fn();
  let onFocusChanged: ((event: { payload: boolean }) => void) | undefined;
  let onThemeChanged: ((event: { payload: "light" | "dark" }) => void) | undefined;
  nativeWindow.onFocusChanged.mockImplementation(
    (handler: (event: { payload: boolean }) => void) => {
      onFocusChanged = handler;
      return Promise.resolve(unlistenFocus);
    },
  );
  nativeWindow.onThemeChanged.mockImplementation(
    (handler: (event: { payload: "light" | "dark" }) => void) => {
      onThemeChanged = handler;
      return Promise.resolve(unlistenTheme);
    },
  );
  const view = render(<AppearanceProbe />);
  await waitFor(() => expect(document.documentElement).toHaveAttribute("data-appearance", "dark"));
  expect(screen.getByTestId("appearance")).toHaveAttribute("data-appearance", "dark");

  nativeAppearance.value = "light";
  act(() => onFocusChanged?.({ payload: true }));
  await waitFor(() => expect(document.documentElement).toHaveAttribute("data-appearance", "light"));
  expect(screen.getByTestId("appearance")).toHaveAttribute("data-appearance", "light");

  act(() => onThemeChanged?.({ payload: "dark" }));
  expect(document.documentElement).toHaveAttribute("data-appearance", "dark");
  expect(screen.getByTestId("appearance")).toHaveAttribute("data-appearance", "dark");

  view.unmount();
  expect(unlistenFocus).toHaveBeenCalledOnce();
  expect(unlistenTheme).toHaveBeenCalledOnce();
});

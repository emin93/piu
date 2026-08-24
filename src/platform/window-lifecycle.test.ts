import { beforeEach, expect, test, vi } from "vitest";

import { listenToWindowClose } from "./window-lifecycle";

type CloseListener = (event: { preventDefault: () => void }) => Promise<void>;

const windowBoundary = vi.hoisted(() => ({
  destroy: vi.fn(),
  onCloseRequested: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowBoundary,
}));

beforeEach(() => {
  windowBoundary.destroy.mockReset();
  windowBoundary.destroy.mockResolvedValue(undefined);
  windowBoundary.onCloseRequested.mockReset();
});

test("waits for draft persistence before destroying the native window", async () => {
  let closeRequested: CloseListener | undefined;
  const unlisten = vi.fn();
  windowBoundary.onCloseRequested.mockImplementation((listener: CloseListener) => {
    closeRequested = listener;
    return Promise.resolve(unlisten);
  });
  const beforeClose = vi.fn().mockResolvedValue(undefined);
  const preventDefault = vi.fn();

  await expect(listenToWindowClose(beforeClose)).resolves.toBe(unlisten);
  await closeRequested?.({ preventDefault });

  expect(preventDefault).toHaveBeenCalledOnce();
  expect(beforeClose).toHaveBeenCalledOnce();
  expect(windowBoundary.destroy).toHaveBeenCalledOnce();
});

test("keeps the native window open when draft persistence fails", async () => {
  let closeRequested: CloseListener | undefined;
  windowBoundary.onCloseRequested.mockImplementation((listener: CloseListener) => {
    closeRequested = listener;
    return Promise.resolve(vi.fn());
  });
  const beforeClose = vi.fn().mockRejectedValue(new Error("storage unavailable"));

  await listenToWindowClose(beforeClose);
  await closeRequested?.({ preventDefault: vi.fn() });

  expect(windowBoundary.destroy).not.toHaveBeenCalled();
});

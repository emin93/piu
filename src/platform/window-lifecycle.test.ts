import { beforeEach, expect, test, vi } from "vitest";

import { listenToWindowClose } from "./window-lifecycle";

type CloseListener = (event: { preventDefault: () => void }) => Promise<void>;

const windowBoundary = vi.hoisted(() => ({
  onCloseRequested: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowBoundary,
}));

beforeEach(() => {
  windowBoundary.onCloseRequested.mockReset();
});

test("prevents native window destruction and delegates application shutdown", async () => {
  let closeRequested: CloseListener | undefined;
  const unlisten = vi.fn();
  windowBoundary.onCloseRequested.mockImplementation((listener: CloseListener) => {
    closeRequested = listener;
    return Promise.resolve(unlisten);
  });
  const resolveRequest = vi.fn().mockResolvedValue(undefined);
  const preventDefault = vi.fn();

  await expect(listenToWindowClose(resolveRequest)).resolves.toBe(unlisten);
  await closeRequested?.({ preventDefault });

  expect(preventDefault).toHaveBeenCalledOnce();
  expect(resolveRequest).toHaveBeenCalledOnce();
});

test("keeps the native window open while the delegated request resolves", async () => {
  let closeRequested: CloseListener | undefined;
  windowBoundary.onCloseRequested.mockImplementation((listener: CloseListener) => {
    closeRequested = listener;
    return Promise.resolve(vi.fn());
  });
  const resolveRequest = vi.fn().mockResolvedValue(undefined);

  await listenToWindowClose(resolveRequest);
  await closeRequested?.({ preventDefault: vi.fn() });

  expect(resolveRequest).toHaveBeenCalledOnce();
});

test("keeps the native window open when close preparation fails", async () => {
  let closeRequested: CloseListener | undefined;
  windowBoundary.onCloseRequested.mockImplementation((listener: CloseListener) => {
    closeRequested = listener;
    return Promise.resolve(vi.fn());
  });
  const resolveRequest = vi.fn().mockRejectedValue(new Error("storage unavailable"));

  await listenToWindowClose(resolveRequest);
  await closeRequested?.({ preventDefault: vi.fn() });

  expect(resolveRequest).toHaveBeenCalledOnce();
});

import { getCurrentWindow } from "@tauri-apps/api/window";

export function listenToWindowClose(beforeClose: () => Promise<void>) {
  const window = getCurrentWindow();
  return window.onCloseRequested(async (event) => {
    event.preventDefault();
    try {
      await beforeClose();
      await window.destroy();
    } catch {
      // The draft status explains the failure and the window stays open for recovery.
    }
  });
}

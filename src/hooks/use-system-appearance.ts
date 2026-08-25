import { getCurrentWindow } from "@tauri-apps/api/window";
import { useLayoutEffect, useState } from "react";

import { readSystemAppearance, type SystemAppearance } from "@/platform/system-appearance";

const DARK_APPEARANCE_QUERY = "(prefers-color-scheme: dark)";
const SYSTEM_APPEARANCE_POLL_MS = 2_000;

export function useSystemAppearance(): SystemAppearance {
  const [appearance, setAppearance] = useState<SystemAppearance>(() =>
    window.matchMedia(DARK_APPEARANCE_QUERY).matches ? "dark" : "light",
  );

  useLayoutEffect(() => {
    document.documentElement.dataset.appearance = appearance;
  }, [appearance]);

  useLayoutEffect(() => {
    const mediaQuery = window.matchMedia(DARK_APPEARANCE_QUERY);
    const handleAppearanceChange = (event: MediaQueryListEvent) =>
      setAppearance(event.matches ? "dark" : "light");

    mediaQuery.addEventListener("change", handleAppearanceChange);

    let disposed = false;
    let nativeRevision = 0;
    const nativeUnlisteners: Array<() => void> = [];
    let appearancePoll: number | undefined;
    let appWindow: ReturnType<typeof getCurrentWindow> | undefined;
    try {
      appWindow = getCurrentWindow();
    } catch {
      appWindow = undefined;
    }
    if (appWindow) {
      const synchronizeNativeAppearance = async () => {
        const revisionBeforeRead = nativeRevision;
        const nativeAppearance = await readSystemAppearance();
        if (!disposed && nativeRevision === revisionBeforeRead) {
          setAppearance(nativeAppearance);
        }
      };
      void synchronizeNativeAppearance().catch(() => undefined);
      appearancePoll = window.setInterval(() => {
        void synchronizeNativeAppearance().catch(() => undefined);
      }, SYSTEM_APPEARANCE_POLL_MS);

      void appWindow
        .onThemeChanged(({ payload }) => {
          nativeRevision += 1;
          if (!disposed) setAppearance(payload);
        })
        .then((unlisten) => {
          if (disposed) unlisten();
          else nativeUnlisteners.push(unlisten);
        })
        .catch(() => undefined);
      void appWindow
        .onFocusChanged(({ payload: focused }) => {
          if (focused) void synchronizeNativeAppearance().catch(() => undefined);
        })
        .then((unlisten) => {
          if (disposed) unlisten();
          else nativeUnlisteners.push(unlisten);
        })
        .catch(() => undefined);
    }

    return () => {
      disposed = true;
      mediaQuery.removeEventListener("change", handleAppearanceChange);
      if (appearancePoll !== undefined) window.clearInterval(appearancePoll);
      for (const unlisten of nativeUnlisteners) unlisten();
    };
  }, []);

  return appearance;
}

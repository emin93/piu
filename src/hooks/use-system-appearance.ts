import { useLayoutEffect } from "react";

const DARK_APPEARANCE_QUERY = "(prefers-color-scheme: dark)";

export function useSystemAppearance(): void {
  useLayoutEffect(() => {
    const mediaQuery = window.matchMedia(DARK_APPEARANCE_QUERY);
    const updateAppearance = (matches: boolean) => {
      document.documentElement.dataset.appearance = matches ? "dark" : "light";
    };
    const handleAppearanceChange = (event: MediaQueryListEvent) => updateAppearance(event.matches);

    updateAppearance(mediaQuery.matches);
    mediaQuery.addEventListener("change", handleAppearanceChange);
    return () => mediaQuery.removeEventListener("change", handleAppearanceChange);
  }, []);
}

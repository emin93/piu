import { vi } from "vitest";

type Appearance = "light" | "dark";

export function installMatchMedia(initialAppearance: Appearance) {
  let appearance = initialAppearance;
  const listeners = new Set<EventListenerOrEventListenerObject>();
  const mediaQueryList = {
    media: "(prefers-color-scheme: dark)",
    onchange: null,
    get matches() {
      return appearance === "dark";
    },
    addEventListener: (_type: string, listener: EventListenerOrEventListenerObject) => {
      listeners.add(listener);
    },
    removeEventListener: (_type: string, listener: EventListenerOrEventListenerObject) => {
      listeners.delete(listener);
    },
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  } satisfies MediaQueryList;
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => mediaQueryList),
  );

  return {
    setAppearance(nextAppearance: Appearance) {
      appearance = nextAppearance;
      const event = Object.assign(new Event("change"), {
        matches: appearance === "dark",
        media: mediaQueryList.media,
      }) as MediaQueryListEvent;
      for (const listener of listeners) {
        if (typeof listener === "function") listener.call(mediaQueryList, event);
        else listener.handleEvent(event);
      }
    },
  };
}

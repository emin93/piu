import { expect, test } from "vitest";

import defaultCapability from "../src-tauri/capabilities/default.json";
import tauriConfig from "../src-tauri/tauri.conf.json";

test("the native traffic lights use the centerline of Più's 52-pixel titlebar", () => {
  expect(tauriConfig.app.windows[0]).toMatchObject({
    hiddenTitle: true,
    titleBarStyle: "Overlay",
    trafficLightPosition: { x: 14, y: 26 },
  });
});

test("the main window is allowed to invoke Tauri's native drag command", () => {
  expect(defaultCapability.windows).toContain("main");
  expect(defaultCapability.permissions).toContain("core:window:allow-start-dragging");
});

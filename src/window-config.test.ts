import { expect, test } from "vitest";

import defaultCapability from "../src-tauri/capabilities/default.json";
import tauriConfig from "../src-tauri/tauri.conf.json";

test("the native traffic lights use Tauri's decorated overlay titlebar", () => {
  expect(tauriConfig.app.windows[0]).toMatchObject({
    decorations: true,
    hiddenTitle: true,
    titleBarStyle: "Overlay",
  });
  expect(tauriConfig.app.windows[0]).not.toHaveProperty("trafficLightPosition");
});

test("the main window is allowed to invoke Tauri's native drag command", () => {
  expect(defaultCapability.windows).toContain("main");
  expect(defaultCapability.permissions).toContain("core:window:allow-start-dragging");
});

import { expect, test } from "vitest";

import tauriConfig from "../src-tauri/tauri.conf.json";

test("the native traffic lights are centered in Più's 52-pixel titlebar", () => {
  expect(tauriConfig.app.windows[0]).toMatchObject({
    hiddenTitle: true,
    titleBarStyle: "Overlay",
    trafficLightPosition: { x: 14, y: 20 },
  });
});

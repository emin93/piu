import { expect, test } from "vitest";

import alertDialogSource from "./components/ui/alert-dialog.tsx?raw";
import dialogSource from "./components/ui/dialog.tsx?raw";
import stylesheet from "./styles.css?raw";

test("the interface uses two local Latin Geist variable faces", () => {
  expect(stylesheet.match(/@font-face/g)).toHaveLength(2);
  expect(stylesheet).toContain("geist-latin-wght-normal.woff2");
  expect(stylesheet).toContain("geist-mono-latin-wght-normal.woff2");
  expect(stylesheet).not.toMatch(/@import\s+url\(["']?https?:/i);
  expect(stylesheet).not.toMatch(/src:\s*url\(["']?https?:/i);
});

test("motion and transparency follow macOS accessibility preferences", () => {
  const reducedMotion = stylesheet.match(
    /@media \(prefers-reduced-motion: reduce\) \{([\s\S]*?)\n\}/,
  )?.[1];
  expect(reducedMotion).toContain("animation-duration: 0.01ms !important");
  expect(reducedMotion).toContain("transition-duration: 0.01ms !important");
  expect(stylesheet).toContain("@media (prefers-reduced-transparency: reduce)");
});

test("the composer layout seam docks through motion that reduced-motion disables", () => {
  expect(stylesheet).toMatch(
    /\.composer-stage\[data-composer-layout="docked"\][\s\S]*?align-self: end/,
  );
  expect(stylesheet).toMatch(/\.composer-stage\s*\{[\s\S]*?transition:/);
  expect(stylesheet).toMatch(
    /@media \(prefers-reduced-motion: reduce\)[\s\S]*?transition-duration: 0\.01ms !important/,
  );
});

test("the modal overlay uses restrained dimming without backdrop blur", () => {
  const overlayRule = stylesheet.match(
    /\[data-slot="alert-dialog-overlay"\],[\s\S]*?\{([^}]*)\}/,
  )?.[1];
  expect(overlayRule).not.toContain("backdrop-filter");
  expect(alertDialogSource).not.toContain("backdrop-blur");
  expect(dialogSource).not.toContain("backdrop-blur");
});

test("the keyboard splitter uses a short focus indicator rather than a full-height stripe", () => {
  expect(stylesheet).toMatch(/\.sidebar-resize-handle::before[\s\S]*?height: 40px/);
  expect(stylesheet).toMatch(/\.sidebar-resize-handle:focus-visible::before[\s\S]*?opacity: 0\.72/);
  expect(stylesheet).not.toMatch(
    /\.sidebar-resize-handle:focus-visible\s*\{[^}]*background: var\(--ring\)/,
  );
});

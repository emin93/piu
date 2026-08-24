import { expect, test } from "vitest";

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

test("the keyboard splitter uses a short focus indicator rather than a full-height stripe", () => {
  expect(stylesheet).toMatch(/\.sidebar-resize-handle::before[\s\S]*?height: 40px/);
  expect(stylesheet).toMatch(/\.sidebar-resize-handle:focus-visible::before[\s\S]*?opacity: 0\.72/);
  expect(stylesheet).not.toMatch(
    /\.sidebar-resize-handle:focus-visible\s*\{[^}]*background: var\(--ring\)/,
  );
});

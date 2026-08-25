import { expect, test } from "vitest";

import alertDialogSource from "./components/ui/alert-dialog.tsx?raw";
import dialogSource from "./components/ui/dialog.tsx?raw";
import modelResourceSource from "./features/model-resources/ModelResourcePanel.tsx?raw";
import modelResourceStyles from "./features/model-resources/model-resource-panel.css?raw";
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

test("the sidebar splitter keeps a wide hit target with only a one-pixel visual divider", () => {
  expect(stylesheet).toMatch(
    /\.sidebar-resize-handle\s*\{[^}]*width: 4px;[^}]*background: transparent;/,
  );
  expect(stylesheet).toMatch(
    /\.sidebar-resize-handle::after\s*\{[^}]*left: 1px;[^}]*width: 1px;[^}]*background: var\(--sidebar-border\);/,
  );
  expect(stylesheet).toMatch(
    /html\[data-inbox-sidebar-resizing\]\s*\{[^}]*cursor: col-resize;[^}]*user-select: none;/,
  );
});

test("native controls and inherited sidebar copy keep explicit system-theme contrast", () => {
  expect(stylesheet).toMatch(/:root\s*\{[^}]*color-scheme: light;/);
  expect(stylesheet).toMatch(/:root\[data-appearance="dark"\]\s*\{[^}]*color-scheme: dark;/);
  expect(stylesheet).not.toMatch(/@media \(prefers-color-scheme: dark\)/);
  expect(stylesheet).toMatch(
    /\.product-composer-input\s*\{[^}]*color: var\(--foreground\);[^}]*caret-color: var\(--foreground\);/,
  );
  expect(stylesheet).toMatch(/\.draft-row-prompt\s*\{[^}]*color: var\(--sidebar-foreground\);/);
});

test("application chrome is nonselectable while editing and transcript content remain selectable", () => {
  expect(stylesheet).toMatch(
    /html,\s*body,\s*#root\s*\{[^}]*-webkit-user-select: none;[^}]*user-select: none;/,
  );
  expect(stylesheet).toMatch(
    /input,\s*textarea,\s*\[contenteditable="true"\],\s*\.conversation-transcript\s*\{[^}]*-webkit-user-select: text;[^}]*user-select: text;/,
  );
  expect(stylesheet).not.toMatch(/body\s*\{[^}]*user-select: text;/);
});

test("long chat branches stay inside their inbox metadata column", () => {
  expect(stylesheet).toMatch(/\.chat-row-copy\s*\{[\s\S]*?display: grid;[\s\S]*?min-width: 0;/);
  expect(stylesheet).toMatch(
    /\.chat-row-project\s*\{[\s\S]*?max-width: 42%;[\s\S]*?overflow: hidden;[\s\S]*?text-overflow: ellipsis;/,
  );
  expect(stylesheet).toMatch(
    /\.chat-row-branch\s*\{[\s\S]*?min-width: 0;[\s\S]*?overflow: hidden;[\s\S]*?text-overflow: ellipsis;/,
  );
});

test("project-scoped idle chats collapse to a compact two-line row", () => {
  expect(stylesheet).toMatch(/\.chat-row\s*\{[^}]*contain-intrinsic-block-size: 62px;/);
  expect(stylesheet).toMatch(
    /\.chat-row\[data-compact="true"\]\s*\{[^}]*contain-intrinsic-block-size: 46px;/,
  );
  expect(stylesheet).toMatch(
    /\.chat-row\[data-compact="true"\] \.chat-row-select\s*\{[^}]*min-height: 46px;/,
  );
  expect(stylesheet).toMatch(
    /\.chat-row\[data-compact="true"\] \.chat-actions-trigger\s*\{[^}]*top: 11px;/,
  );
});

test("model resources compose the shared controls without a second custom control system", () => {
  expect(modelResourceSource).not.toMatch(/<(?:button|input)\b/);
  expect(modelResourceSource).toContain("<Button");
  expect(modelResourceSource).toContain("<Input");
  expect(modelResourceSource).toContain("<Badge");
  expect(modelResourceSource).toContain("<AlertDialog");
  expect(modelResourceSource).toContain("<Skeleton");
  expect(modelResourceStyles).toMatch(/\.model-resource-panel\s*\{[\s\S]*?gap: 16px/);
});

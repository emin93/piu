import { readdir, readFile, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { gzipSync } from "node:zlib";

const budgets = {
  css: { raw: 64 * 1024, gzip: 16 * 1024 },
  javascript: { raw: 512 * 1024, gzip: 160 * 1024 },
};

const manifestPath = resolve("dist/.vite/manifest.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
const entry = Object.values(manifest).find((chunk) => chunk.isEntry);
if (!entry) throw new Error(`No entry chunk found in ${manifestPath}`);

const expectedDeferredSurfaces = [
  "ConversationSurface",
  "DiffSurface",
  "FilesSurface",
  "SettingsSurface",
  "TerminalSurface",
];
const dynamicImports = entry.dynamicImports ?? [];
for (const surface of expectedDeferredSurfaces) {
  const dynamicEntry = dynamicImports.find((path) => path.includes(`/${surface}.tsx`));
  if (!dynamicEntry || !manifest[dynamicEntry]?.isDynamicEntry) {
    throw new Error(`${surface} must remain outside the initial JavaScript bundle`);
  }
}

const entryPath = resolve("dist", entry.file);
const entrySource = await readFile(entryPath);
const entryBytes = (await stat(entryPath)).size;
const entryGzipBytes = gzipSync(entrySource).byteLength;
if (entryBytes > budgets.javascript.raw || entryGzipBytes > budgets.javascript.gzip) {
  throw new Error(
    `Opening-route JavaScript exceeds its budget: ${entryBytes} B raw / ${entryGzipBytes} B gzip`,
  );
}

const cssFiles = entry.css ?? [];
if (cssFiles.length !== 1) {
  throw new Error(`Expected one opening-route stylesheet, received ${cssFiles.length}`);
}
const cssPath = resolve("dist", cssFiles[0]);
const cssSource = await readFile(cssPath);
const cssBytes = (await stat(cssPath)).size;
const cssGzipBytes = gzipSync(cssSource).byteLength;
if (cssBytes > budgets.css.raw || cssGzipBytes > budgets.css.gzip) {
  throw new Error(
    `Opening-route CSS exceeds its budget: ${cssBytes} B raw / ${cssGzipBytes} B gzip`,
  );
}
if (/url\(\s*["']?https?:\/\//i.test(cssSource.toString("utf8"))) {
  throw new Error("Opening-route CSS must not request fonts or other assets from the network");
}

const assetFiles = await readdir(resolve("dist/assets"));
const fontFiles = assetFiles.filter((file) => file.endsWith(".woff2")).sort();
if (
  fontFiles.length !== 2 ||
  !fontFiles.some((file) => file.startsWith("geist-latin-wght-normal-")) ||
  !fontFiles.some((file) => file.startsWith("geist-mono-latin-wght-normal-"))
) {
  throw new Error(`Expected exactly the two local Latin Geist variable fonts: ${fontFiles}`);
}

console.log(
  JSON.stringify(
    {
      entry: entry.file,
      entryBytes,
      entryGzipBytes,
      css: cssFiles[0],
      cssBytes,
      cssGzipBytes,
      fontFiles,
      budgets,
      deferredSurfaces: expectedDeferredSurfaces,
    },
    null,
    2,
  ),
);

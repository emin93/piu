import { readFile, stat } from "node:fs/promises";
import { resolve } from "node:path";

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

const entryBytes = (await stat(resolve("dist", entry.file))).size;
console.log(
  JSON.stringify(
    {
      entry: entry.file,
      entryBytes,
      deferredSurfaces: expectedDeferredSurfaces,
    },
    null,
    2,
  ),
);

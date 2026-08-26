import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";
import { defineConfig } from "vite";

const repositoryRoot = resolve(import.meta.dirname, "../..");

export default defineConfig({
  build: {
    emptyOutDir: true,
    outDir: resolve(repositoryRoot, "work/chat-performance-dist"),
    sourcemap: true,
    target: "safari18",
  },
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": resolve(repositoryRoot, "src"),
      "#inbox-performance-review": resolve(import.meta.dirname, "chat/inbox-performance-review.ts"),
      "#model-resource-qa": resolve(
        repositoryRoot,
        "src/features/model-resources/model-resource-qa.production.tsx",
      ),
    },
  },
  root: resolve(import.meta.dirname, "chat"),
});

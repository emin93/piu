import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";
import { loadEnv } from "vite";
import { defineConfig } from "vitest/config";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const qaEntry =
    env.VITE_PIU_MODEL_QA_GALLERY === "1"
      ? "src/features/model-resources/model-resource-qa.gallery.tsx"
      : "src/features/model-resources/model-resource-qa.production.tsx";

  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        "@": resolve(import.meta.dirname, "src"),
        "#model-resource-qa": resolve(import.meta.dirname, qaEntry),
      },
    },
    clearScreen: false,
    server: {
      strictPort: true,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
    },
    build: {
      manifest: true,
      sourcemap: true,
      target: "safari18",
    },
    test: {
      environment: "jsdom",
      setupFiles: ["./src/test/setup.ts"],
      css: true,
    },
  };
});

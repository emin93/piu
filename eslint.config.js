import eslint from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: ["dist", "node_modules", "src/generated", "src-tauri/gen", "src-tauri/target", "work"],
  },
  eslint.configs.recommended,
  {
    files: ["scripts/**/*.mjs", "src-tauri/vendor/agent-runtime/**/*.mjs", "eslint.config.js"],
    languageOptions: { globals: globals.node },
  },
  {
    files: ["scripts/performance/**/*.{ts,tsx}", "src/**/*.{ts,tsx}", "vite.config.ts"],
    extends: [...tseslint.configs.recommendedTypeChecked],
    languageOptions: {
      ecmaVersion: 2022,
      globals: {
        ...globals.browser,
        ...globals.node,
      },
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.flat.recommended.rules,
      ...reactRefresh.configs.vite.rules,
    },
  },
  {
    files: ["scripts/performance/**/*.{ts,tsx}"],
    rules: {
      "react-refresh/only-export-components": "off",
    },
  },
);

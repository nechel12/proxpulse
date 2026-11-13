import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import tseslint from "typescript-eslint";

export default defineConfig(
  { ignores: ["dist/**", "node_modules/**", "src-tauri/**"] },
  js.configs.recommended,
  tseslint.configs.recommended,
  {
    files: ["src/**/*.ts"],
    rules: {
      "no-undef": "off",
      "no-empty": ["error", { allowEmptyCatch: true }],
      "@typescript-eslint/no-unused-vars": "off",
      "@typescript-eslint/no-explicit-any": "error",
    },
  },
  {
    // Release and version scripts run on Node.
    files: ["scripts/**/*.mjs"],
    languageOptions: { globals: { process: "readonly", console: "readonly", URL: "readonly" } },
  },
  {
    // The demo backend runs in the page.
    files: ["scripts/demo/**/*.js"],
    languageOptions: {
      globals: {
        window: "readonly", document: "readonly", location: "readonly", localStorage: "readonly",
        URLSearchParams: "readonly", setTimeout: "readonly", Event: "readonly",
      },
    },
  },
);

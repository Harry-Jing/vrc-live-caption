// @ts-check
import { globalIgnores } from "eslint/config";
import js from "@eslint/js";
import {
  configureVueProject,
  defineConfigWithVueTs,
  vueTsConfigs,
} from "@vue/eslint-config-typescript";
import skipFormatting from "eslint-config-prettier/flat";
import pluginVue from "eslint-plugin-vue";

configureVueProject({
  rootDir: import.meta.dirname,
});

export default defineConfigWithVueTs(
  {
    name: "app/files-to-lint",
    files: ["src/**/*.{vue,ts}"],
  },

  globalIgnores(
    [
      "node_modules/**",
      "dist/**",
      "dist-ssr/**",
      "src-tauri/**",
      "auto-imports.d.ts",
      "components.d.ts",
      ".tmp/**",
    ],
    "app/global-ignores",
  ),

  {
    name: "app/linter-options",
    linterOptions: {
      reportUnusedDisableDirectives: "error",
      reportUnusedInlineConfigs: "error",
    },
  },

  js.configs.recommended,
  ...pluginVue.configs["flat/recommended-error"],
  vueTsConfigs.strictTypeChecked,

  {
    name: "app/type-imports",
    files: ["src/**/*.{vue,ts}"],
    rules: {
      "@typescript-eslint/consistent-type-imports": [
        "error",
        {
          fixStyle: "inline-type-imports",
          prefer: "type-imports",
        },
      ],
    },
  },

  {
    name: "app/layer-boundaries",
    files: ["src/**/*.{vue,ts}"],
    ignores: ["src/runtime/**"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@tauri-apps/*", "@tauri-apps/**"],
              message:
                "Tauri APIs must stay behind src/runtime/. Use the runtime context instead.",
            },
            {
              group: [
                "**/runtime/backend",
                "**/tauriBackend",
                "**/previewBackend",
              ],
              message:
                "Backend implementations are internal to src/runtime/. Use useRuntime or the runtime context instead.",
            },
          ],
        },
      ],
    },
  },

  {
    name: "app/no-debug-output",
    files: ["src/**/*.{vue,ts}"],
    rules: {
      "no-console": ["error", { allow: ["warn", "error"] }],
    },
  },

  skipFormatting,
);

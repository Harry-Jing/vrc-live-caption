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

  skipFormatting,
);

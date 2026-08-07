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
      ".local/**",
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
    name: "app/vue-component-shape",
    files: ["src/**/*.vue"],
    rules: {
      "vue/block-lang": ["error", { script: { lang: "ts" } }],
      "vue/component-api-style": ["error", ["script-setup"]],
      "vue/component-name-in-template-casing": [
        "error",
        "PascalCase",
        { registeredComponentsOnly: false },
      ],
      "vue/no-bare-strings-in-template": [
        "error",
        {
          attributes: {
            "/.+/": [
              "title",
              "label",
              "description",
              "placeholder",
              "alt",
              "aria-label",
              "aria-placeholder",
              "aria-roledescription",
              "aria-valuetext",
            ],
          },
        },
      ],
    },
  },

  {
    name: "app/layer-boundaries",
    files: ["src/**/*.{vue,ts}", "tests/eslint/**/*.mjs"],
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
              regex:
                "(^|/)runtime/(backend|tauriBackend|previewBackend)(\\.[cm]?[jt]sx?)?$",
              message:
                "Backend implementations are internal to src/runtime/. Use useRuntime or the runtime context instead.",
            },
          ],
        },
      ],
      "no-restricted-syntax": [
        "error",
        {
          selector: "ImportExpression[source.value=/^@tauri-apps(?:\\/|$)/]",
          message:
            "Tauri APIs must stay behind src/runtime/. Use the runtime context instead.",
        },
        {
          selector:
            "ImportExpression > TemplateLiteral[expressions.length=0] > TemplateElement[value.cooked=/^@tauri-apps(?:\\/|$)/]",
          message:
            "Tauri APIs must stay behind src/runtime/. Use the runtime context instead.",
        },
        {
          selector:
            "ImportExpression[source.value=/(?:^|\\/)runtime\\/(?:backend|tauriBackend|previewBackend)(?:\\.[cm]?[jt]sx?)?$/]",
          message:
            "Backend implementations are internal to src/runtime/. Use useRuntime or the runtime context instead.",
        },
        {
          selector:
            "ImportExpression > TemplateLiteral[expressions.length=0] > TemplateElement[value.cooked=/(?:^|\\/)runtime\\/(?:backend|tauriBackend|previewBackend)(?:\\.[cm]?[jt]sx?)?$/]",
          message:
            "Backend implementations are internal to src/runtime/. Use useRuntime or the runtime context instead.",
        },
        {
          selector:
            "CallExpression[callee.object.name='toast'][callee.property.name='add'] > ObjectExpression > Property[key.name=/^(title|description)$/] > Literal",
          message:
            "Toast copy must be resolved through uiText so it has a stable message key.",
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

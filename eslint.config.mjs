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

const restrictedImports = {
  tauri: {
    group: ["@tauri-apps/*", "@tauri-apps/**"],
    message:
      "Tauri APIs must stay behind src/platform/. Use a platform interface instead.",
  },
  runtimeAdapters: {
    regex: "(^|/)((platform/)?tauri|preview)(/|$)",
    message:
      "Runtime adapter implementations are internal. Use the runtime composition interface instead.",
  },
  tauriAdapter: {
    regex: "(^|/)(platform/)?tauri(/|$)",
    message: "The Tauri runtime adapter is internal to platform composition.",
  },
  runtimeFacade: {
    regex: "(^|/)platform/appGateway(\\.[cm]?[jt]sx?)?$",
    message: "Only useRuntime may create the AppGateway.",
  },
  runtimeComposition: {
    regex: "(^|/)(runtime/gateway|platform/appGateway)(\\.[cm]?[jt]sx?)?$",
    message:
      "Runtime composition is internal. Use useRuntime or the runtime context instead.",
  },
  wire: {
    regex: "(^|/)runtime/wire(?:/|$)",
    message:
      "Wire contracts are internal to runtime adapters. Use typed runtime modules or the runtime context instead.",
  },
};

const restrictedImportSyntax = {
  tauri: [
    {
      selector: "ImportExpression[source.value=/^@tauri-apps(?:\\/|$)/]",
      message:
        "Tauri APIs must stay behind src/platform/. Use a platform interface instead.",
    },
    {
      selector:
        "ImportExpression > TemplateLiteral[expressions.length=0] > TemplateElement[value.cooked=/^@tauri-apps(?:\\/|$)/]",
      message:
        "Tauri APIs must stay behind src/platform/. Use a platform interface instead.",
    },
  ],
  runtimeAdapters: [
    {
      selector:
        "ImportExpression[source.value=/(?:^|\\/)(?:(?:platform\\/)?tauri|preview)(?:\\/|$)/]",
      message:
        "Runtime adapter implementations are internal. Use the runtime composition interface instead.",
    },
    {
      selector:
        "ImportExpression > TemplateLiteral[expressions.length=0] > TemplateElement[value.cooked=/(?:^|\\/)(?:(?:platform\\/)?tauri|preview)(?:\\/|$)/]",
      message:
        "Runtime adapter implementations are internal. Use the runtime composition interface instead.",
    },
  ],
  tauriAdapter: [
    {
      selector:
        "ImportExpression[source.value=/(?:^|\\/)(?:platform\\/)?tauri(?:\\/|$)/]",
      message: "The Tauri runtime adapter is internal to platform composition.",
    },
    {
      selector:
        "ImportExpression > TemplateLiteral[expressions.length=0] > TemplateElement[value.cooked=/(?:^|\\/)(?:platform\\/)?tauri(?:\\/|$)/]",
      message: "The Tauri runtime adapter is internal to platform composition.",
    },
  ],
  runtimeFacade: [
    {
      selector:
        "ImportExpression[source.value=/(?:^|\\/)platform\\/appGateway(?:\\.[cm]?[jt]sx?)?$/]",
      message: "Only useRuntime may create the AppGateway.",
    },
    {
      selector:
        "ImportExpression > TemplateLiteral[expressions.length=0] > TemplateElement[value.cooked=/(?:^|\\/)platform\\/appGateway(?:\\.[cm]?[jt]sx?)?$/]",
      message: "Only useRuntime may create the AppGateway.",
    },
  ],
  runtimeComposition: [
    {
      selector:
        "ImportExpression[source.value=/(?:^|\\/)(?:runtime\\/gateway|platform\\/appGateway)(?:\\.[cm]?[jt]sx?)?$/]",
      message:
        "Runtime composition is internal. Use useRuntime or the runtime context instead.",
    },
    {
      selector:
        "ImportExpression > TemplateLiteral[expressions.length=0] > TemplateElement[value.cooked=/(?:^|\\/)(?:runtime\\/gateway|platform\\/appGateway)(?:\\.[cm]?[jt]sx?)?$/]",
      message:
        "Runtime composition is internal. Use useRuntime or the runtime context instead.",
    },
  ],
  wire: [
    {
      selector:
        "ImportExpression[source.value=/(?:^|\\/)runtime\\/wire(?:\\/|$)/]",
      message:
        "Wire contracts are internal to runtime adapters. Use typed runtime modules or the runtime context instead.",
    },
    {
      selector:
        "ImportExpression > TemplateLiteral[expressions.length=0] > TemplateElement[value.cooked=/(?:^|\\/)runtime\\/wire(?:\\/|$)/]",
      message:
        "Wire contracts are internal to runtime adapters. Use typed runtime modules or the runtime context instead.",
    },
  ],
};

const stableToastCopyRestriction = {
  selector:
    "CallExpression[callee.object.name='toast'][callee.property.name='add'] > ObjectExpression > Property[key.name=/^(title|description)$/] > Literal",
  message:
    "Toast copy must be resolved through uiText so it has a stable message key.",
};

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
    name: "app/runtime-adapter-seam",
    files: ["src/runtime/**/*.{vue,ts}"],
    ignores: ["src/runtime/useRuntime.ts"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            restrictedImports.tauri,
            restrictedImports.runtimeAdapters,
            restrictedImports.runtimeFacade,
          ],
        },
      ],
      "no-restricted-syntax": [
        "error",
        ...restrictedImportSyntax.tauri,
        ...restrictedImportSyntax.runtimeAdapters,
        ...restrictedImportSyntax.runtimeFacade,
      ],
    },
  },

  {
    name: "app/runtime-boundary-fixtures",
    files: ["tests/eslint/runtime/**/*.mjs"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            restrictedImports.tauri,
            restrictedImports.runtimeAdapters,
            restrictedImports.runtimeFacade,
          ],
        },
      ],
      "no-restricted-syntax": [
        "error",
        ...restrictedImportSyntax.tauri,
        ...restrictedImportSyntax.runtimeAdapters,
        ...restrictedImportSyntax.runtimeFacade,
        stableToastCopyRestriction,
      ],
    },
  },

  {
    name: "app/runtime-composition-entry",
    files: ["src/runtime/useRuntime.ts"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            restrictedImports.tauri,
            restrictedImports.runtimeAdapters,
          ],
        },
      ],
      "no-restricted-syntax": [
        "error",
        ...restrictedImportSyntax.tauri,
        ...restrictedImportSyntax.runtimeAdapters,
      ],
    },
  },

  {
    name: "app/preview-adapter-seam",
    files: [
      "src/platform/preview/**/*.{vue,ts}",
      "tests/eslint/preview/**/*.mjs",
    ],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            restrictedImports.tauri,
            restrictedImports.tauriAdapter,
            restrictedImports.runtimeFacade,
            restrictedImports.wire,
          ],
        },
      ],
      "no-restricted-syntax": [
        "error",
        ...restrictedImportSyntax.tauri,
        ...restrictedImportSyntax.tauriAdapter,
        ...restrictedImportSyntax.runtimeFacade,
        ...restrictedImportSyntax.wire,
        stableToastCopyRestriction,
      ],
    },
  },

  {
    name: "app/ui-runtime-seams",
    files: ["src/**/*.{vue,ts}", "tests/eslint/*.mjs"],
    ignores: [
      "src/runtime/**",
      "src/platform/**",
      "tests/eslint/runtime/**",
      "tests/eslint/preview/**",
      "tests/eslint/platform/**",
    ],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            restrictedImports.tauri,
            restrictedImports.runtimeAdapters,
            restrictedImports.runtimeComposition,
            restrictedImports.wire,
          ],
        },
      ],
      "no-restricted-syntax": [
        "error",
        ...restrictedImportSyntax.tauri,
        ...restrictedImportSyntax.runtimeAdapters,
        ...restrictedImportSyntax.runtimeComposition,
        ...restrictedImportSyntax.wire,
        stableToastCopyRestriction,
      ],
    },
  },

  {
    name: "app/platform-adapters",
    files: ["src/platform/**/*.{vue,ts}", "tests/eslint/platform/**/*.mjs"],
    rules: {
      "no-restricted-syntax": ["error", stableToastCopyRestriction],
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

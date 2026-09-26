export default {
  extends: ["@commitlint/config-conventional"],
  rules: {
    // A warning, not an error: the list keeps scopes consistent without blocking a new area.
    "scope-enum": [
      1,
      "always",
      [
        "audio",
        "recognition",
        "translation",
        "chatbox",
        "runtime",
        "settings",
        "credentials",
        "network",
        "diagnostics",
        "ui",
        "desktop",
        "deps",
        "deps-dev",
        "ci",
        "github",
      ],
    ],
  },
};

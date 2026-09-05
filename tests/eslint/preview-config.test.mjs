import { readFile } from "node:fs/promises";
import { fileURLToPath, URL } from "node:url";

import { ESLint } from "eslint";
import { expect, test } from "vitest";

const projectRoot = fileURLToPath(new URL("../../", import.meta.url));
const eslint = new ESLint({ cwd: projectRoot, allowInlineConfig: false });
const sourcePath = "src/platform/preview/appGateway.ts";
const fixturePath = "tests/eslint/preview/layer-boundaries.contract.mjs";
const boundaryRules = ["no-restricted-imports", "no-restricted-syntax"];

test("Preview source and fixture retain the same effective boundary rules", async () => {
  const sourceConfig = await eslint.calculateConfigForFile(sourcePath);
  const fixtureConfig = await eslint.calculateConfigForFile(fixturePath);

  // Fixtures must exercise the rules left after every source scope is composed.
  for (const rule of boundaryRules) {
    expect(sourceConfig.rules[rule], rule).toEqual(fixtureConfig.rules[rule]);
  }
});

test.each([sourcePath, fixturePath])(
  "%s rejects every Preview boundary import",
  async (filePath) => {
    const forbiddenImports = await readFile(
      new URL("preview/layer-boundaries.contract.mjs", import.meta.url),
      "utf8",
    );
    const [result] = await eslint.lintText(forbiddenImports, { filePath });
    expect(result.fatalErrorCount).toBe(0);
    expect(
      result.messages
        .filter(({ ruleId }) => boundaryRules.includes(ruleId))
        .map(({ ruleId }) => ruleId),
    ).toEqual([
      ...Array(4).fill("no-restricted-imports"),
      ...Array(8).fill("no-restricted-syntax"),
    ]);
  },
);

test.each([
  "src/platform/tauri/appGateway.ts",
  "src/platform/confirmation.ts",
  "tests/eslint/platform/layer-boundaries.allowed.mjs",
])("%s retains access to native platform APIs", async (filePath) => {
  const allowedTauriImports = [
    'import "@tauri-apps/api/core";',
    'void import("@tauri-apps/api/core");',
    "void import(`@tauri-apps/api/core`);",
  ].join("\n");
  const [result] = await eslint.lintText(allowedTauriImports, { filePath });
  expect(result.fatalErrorCount).toBe(0);
  expect(
    result.messages.filter(({ ruleId }) => boundaryRules.includes(ruleId)),
  ).toEqual([]);
});

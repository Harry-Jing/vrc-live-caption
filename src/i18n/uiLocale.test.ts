import { expect, test } from "vitest";
import { uiLocaleFromLanguage, uiLocaleFromPreview } from "./uiLocale";

test.each([
  ["zh", "zh-Hans"],
  ["zh-CN", "zh-Hans"],
  ["zh-Hans-CN", "zh-Hans"],
  ["zh-SG", "zh-Hans"],
  ["en-US", "en"],
  ["ja-JP", "en"],
  ["", "en"],
] as const)("maps %s to the supported UI locale %s", (language, expected) => {
  expect(uiLocaleFromLanguage(language)).toBe(expected);
});

test("accepts only supported Preview locale overrides", () => {
  expect(uiLocaleFromPreview("?uiLocale=zh-Hans", "en-US")).toBe("zh-Hans");
  expect(uiLocaleFromPreview("?uiLocale=en", "zh-CN")).toBe("en");
  expect(uiLocaleFromPreview("?uiLocale=ja", "zh-CN")).toBe("zh-Hans");
});

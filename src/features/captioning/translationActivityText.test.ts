import { expect, test } from "vitest";
import {
  translationActivityText,
  translationFailureText,
} from "./translationActivityText";

test("provides Translation activity text in both supported locales", () => {
  expect(translationActivityText("en", "pending")).toBe("Translating");
  expect(translationActivityText("zh-Hans", "pending")).toBe("翻译中");
  expect(translationFailureText("en", "translation.provider_unavailable")).toBe(
    "The Translation service was unavailable.",
  );
  expect(
    translationFailureText("zh-Hans", "translation.provider_unavailable"),
  ).toBe("翻译服务暂时不可用。");
});

test("keeps stable failure reasons provider-neutral and secret-free", () => {
  const rendered = translationFailureText(
    "en",
    "translation.provider_authentication_failed",
  );

  expect(rendered).not.toMatch(/https?:|api[_ -]?key|bearer|provider body/iu);
});

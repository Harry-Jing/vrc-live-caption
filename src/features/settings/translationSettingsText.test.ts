import { expect, test } from "vitest";
import { translationSettingsText } from "./translationSettingsText";

test("provides the Translation controls in English and Simplified Chinese", () => {
  expect(translationSettingsText("en", "contentLabel")).toBe(
    "Completed content",
  );
  expect(translationSettingsText("zh-Hans", "contentLabel")).toBe("已完成内容");
  expect(translationSettingsText("en", "customUploadDisclosure")).toContain(
    "retention policy",
  );
  expect(
    translationSettingsText("zh-Hans", "customUploadDisclosure"),
  ).toContain("数据保留策略");
});

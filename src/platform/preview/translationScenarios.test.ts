import { describe, expect, test } from "vitest";
import {
  createPreviewTranslationScenarioSeed,
  PREVIEW_TRANSLATION_SCENARIOS,
  previewTranslationScenarioFromSearch,
} from "./translationScenarios";

describe("Preview Translation scenarios", () => {
  test.each(PREVIEW_TRANSLATION_SCENARIOS)(
    "creates a contract-valid deterministic %s snapshot",
    (scenario) => {
      const first = createPreviewTranslationScenarioSeed(scenario);
      const second = createPreviewTranslationScenarioSeed(scenario);

      expect(first).toEqual(second);
      expect(first.captionAggregate.contractVersion).toBe(2);
    },
  );

  test("covers Official, Custom, Stop, and replacement generations", () => {
    const official = createPreviewTranslationScenarioSeed("official-success");
    const custom = createPreviewTranslationScenarioSeed("custom-success");
    const stopped = createPreviewTranslationScenarioSeed("stopped");
    const restarted = createPreviewTranslationScenarioSeed("restarted");

    expect(official.config.translation?.endpoint.kind).toBe("official");
    expect(custom.config.translation?.endpoint.kind).toBe("custom");
    expect(custom.generation?.credentials.map(({ id }) => id)).toEqual([
      "openai",
      "customTranslation",
    ]);
    expect(stopped.generation).toBeNull();
    expect(stopped.captionAggregate.activeStream).toBeNull();
    expect(restarted.generation?.id).toBe(8);
    expect(
      restarted.captionAggregate.captions.some(
        ({ generation }) => generation === 7,
      ),
    ).toBe(true);
  });

  test("accepts only the closed query vocabulary", () => {
    expect(
      previewTranslationScenarioFromSearch(
        "?translationScenario=official-pending",
      ),
    ).toBe("official-pending");
    expect(
      previewTranslationScenarioFromSearch(
        "?translationScenario=provider-secret-message",
      ),
    ).toBeNull();
    expect(previewTranslationScenarioFromSearch("")).toBeNull();
  });
});

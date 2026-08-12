import { describe, expect, test } from "vitest";
import { decodeCaptionAggregateSnapshot } from "../runtime/wire/captionAggregateContract";
import { decodeRuntimeControlSnapshot } from "../runtime/wire/runtimeControlContract";
import { createPreviewAppGateway } from "./preview/appGateway";
import { PREVIEW_TRANSLATION_SCENARIOS } from "./preview/translationScenarios";

describe("preview Translation control contract", () => {
  test.each(PREVIEW_TRANSLATION_SCENARIOS)(
    "exposes contract-valid %s pull snapshots",
    async (scenario) => {
      const gateway = createPreviewAppGateway(
        `?translationScenario=${scenario}`,
      );
      const [control, aggregate] = await Promise.all([
        gateway.getRuntimeControlSnapshot(),
        gateway.getCaptionAggregateSnapshot(),
      ]);

      expect(() =>
        decodeRuntimeControlSnapshot(JSON.parse(JSON.stringify(control))),
      ).not.toThrow();
      expect(() =>
        decodeCaptionAggregateSnapshot(JSON.parse(JSON.stringify(aggregate))),
      ).not.toThrow();
    },
  );
});

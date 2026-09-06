import { describe, expect, test } from "vitest";
import captionAggregateFixture from "../../contracts/caption-aggregate-snapshot-v2.json?raw";
import runtimeControlFixture from "../../contracts/runtime-control-snapshot-v3.json?raw";
import {
  TRANSLATION_FAILURE_REASONS,
  type CaptionAggregateSnapshot,
  type SourceSnapshotRef,
} from "./captionAggregate";
import type { ContentSelection } from "./captionPipeline";
import type {
  RuntimeGenerationSnapshot,
  RuntimeGenerationTranslationState,
  RuntimeControlSnapshot,
} from "./runtimeControl";
import { translationPresentation } from "./translationPresentation";

const fixtureAggregate = JSON.parse(
  captionAggregateFixture,
) as CaptionAggregateSnapshot;
const fixtureControl = JSON.parse(
  runtimeControlFixture,
) as RuntimeControlSnapshot;

function translationGeneration(
  content: Exclude<ContentSelection, "sourceOnly">,
  translationState: RuntimeGenerationTranslationState = { state: "active" },
  id = 7,
): RuntimeGenerationSnapshot {
  const generation = fixtureControl.generation;
  const translation = fixtureControl.desired.config.translation;
  if (generation === null || translation === null) {
    throw new Error("Translation fixtures require a generation and selection.");
  }

  return {
    ...generation,
    id,
    phase: "running",
    selection: {
      ...generation.selection,
      publication: { mode: "completed", content },
      translation,
    },
    translationState,
    uploadsSourceText: true,
  };
}

function replaceSourceRefScope(
  sourceRef: SourceSnapshotRef | null,
  generation: number,
  streamId: string,
) {
  return sourceRef === null ? null : { ...sourceRef, generation, streamId };
}

function aggregateInScope(
  aggregate: CaptionAggregateSnapshot,
  generation: number,
  streamId: string,
): CaptionAggregateSnapshot {
  return {
    ...aggregate,
    activeStream: { generation, streamId },
    captions: aggregate.captions.map((caption) => ({
      ...caption,
      generation,
      streamId,
      sourceRef: replaceSourceRefScope(caption.sourceRef, generation, streamId),
    })),
    translationUnits: aggregate.translationUnits.map((outcome) => ({
      ...outcome,
      sourceRef: {
        ...outcome.sourceRef,
        generation,
        streamId,
      },
    })),
  };
}

describe("translation presentation", () => {
  test("is inactive without a runtime generation", () => {
    expect(translationPresentation(null, fixtureAggregate)).toEqual({
      state: "inactive",
      content: null,
      target: null,
      endpointKind: null,
      reasonCode: null,
      units: [],
    });
  });

  test("keeps Source-only inactive without implying retained Translation activity", () => {
    const generation = fixtureControl.generation;
    if (generation === null) {
      throw new Error("Runtime fixture requires a generation.");
    }

    const presentation = translationPresentation(generation, fixtureAggregate);

    expect(presentation).toEqual({
      state: "inactive",
      content: "sourceOnly",
      target: null,
      endpointKind: null,
      reasonCode: null,
      units: [],
    });
    expect(JSON.stringify(presentation)).not.toContain("translation.provider");
    expect(JSON.stringify(presentation)).not.toContain("已完成翻译");
  });

  test("correlates pending, completed, and failed bilingual units to exact Source snapshots", () => {
    const presentation = translationPresentation(
      translationGeneration("bilingual"),
      fixtureAggregate,
    );

    expect(presentation).toMatchObject({
      state: "active",
      content: "bilingual",
      target: "zh-Hans",
      endpointKind: "custom",
      reasonCode: null,
    });
    expect(presentation.units).toEqual([
      {
        state: "failed",
        sourceRef: {
          generation: 7,
          streamId: "recognition-7-1",
          unitId: "speech-7-3",
          revision: 1,
        },
        source: {
          text: "Source whose translation failed.",
          language: "en",
        },
        translation: null,
        reasonCode: "translation.provider_unavailable",
      },
      {
        state: "pending",
        sourceRef: {
          generation: 7,
          streamId: "recognition-7-1",
          unitId: "speech-7-2",
          revision: 1,
        },
        source: {
          text: "Source awaiting translation.",
          language: "en",
        },
        translation: null,
        reasonCode: null,
      },
      {
        state: "completed",
        sourceRef: {
          generation: 7,
          streamId: "recognition-7-1",
          unitId: "speech-7-1",
          revision: 1,
        },
        source: {
          text: "Source with a completed translation.",
          language: "en",
        },
        translation: { text: "已完成翻译的原文。", language: "zh-Hans" },
        reasonCode: null,
      },
    ]);
  });

  test("projects Translation-only with its exact Source pairing and keeps failed units terminal", () => {
    const presentation = translationPresentation(
      translationGeneration("translationOnly"),
      fixtureAggregate,
    );

    expect(presentation.units.map((unit) => unit.state)).toEqual([
      "failed",
      "pending",
      "completed",
    ]);
    expect(presentation.units.map((unit) => unit.source.text)).toEqual([
      "Source whose translation failed.",
      "Source awaiting translation.",
      "Source with a completed translation.",
    ]);
    expect(presentation.units[0]).toMatchObject({
      state: "failed",
      translation: null,
      reasonCode: "translation.provider_unavailable",
    });
    expect(presentation.units[1]).toMatchObject({
      state: "pending",
      translation: null,
      reasonCode: null,
    });
    expect(presentation.units[2]).toMatchObject({
      state: "completed",
      translation: { text: "已完成翻译的原文。" },
      reasonCode: null,
    });
  });

  test("is inactive for a retained generation that is no longer active", () => {
    const failedGeneration: RuntimeGenerationSnapshot = {
      ...translationGeneration("bilingual", {
        state: "degraded",
        reasonCode: "translation.provider_unavailable",
      }),
      phase: "error",
    };

    expect(translationPresentation(failedGeneration, fixtureAggregate)).toEqual(
      {
        state: "inactive",
        content: null,
        target: null,
        endpointKind: null,
        reasonCode: null,
        units: [],
      },
    );
  });

  test("takes generation degradation from Runtime Control on pull or reconnect", () => {
    const generation = translationGeneration("bilingual", {
      state: "degraded",
      reasonCode: "translation.deadline_exceeded",
    });
    const pulled = JSON.parse(
      captionAggregateFixture,
    ) as CaptionAggregateSnapshot;

    expect(translationPresentation(generation, pulled)).toEqual(
      translationPresentation(generation, fixtureAggregate),
    );
    expect(translationPresentation(generation, pulled)).toMatchObject({
      state: "degraded",
      reasonCode: "translation.deadline_exceeded",
    });
  });

  test.each(TRANSLATION_FAILURE_REASONS)(
    "preserves only the stable failure reason %s",
    (reasonCode) => {
      const failedAggregate: CaptionAggregateSnapshot = {
        ...fixtureAggregate,
        captions: fixtureAggregate.captions.filter(
          (caption) => caption.lane === "source",
        ),
        translationUnits: fixtureAggregate.translationUnits.map((outcome) =>
          outcome.state === "completed"
            ? { state: "failed", sourceRef: outcome.sourceRef, reasonCode }
            : outcome.state === "failed"
              ? { ...outcome, reasonCode }
              : outcome,
        ),
      };
      const presentation = translationPresentation(
        translationGeneration("translationOnly", {
          state: "degraded",
          reasonCode,
        }),
        failedAggregate,
      );

      expect(presentation.reasonCode).toBe(reasonCode);
      expect(
        presentation.units
          .filter((unit) => unit.state === "failed")
          .map((unit) => unit.reasonCode),
      ).toEqual([reasonCode, reasonCode]);
    },
  );

  test("redacts endpoint URL and credentials from Translation-only presentation", () => {
    const generation = translationGeneration("translationOnly");
    const presentationText = JSON.stringify(
      translationPresentation(generation, fixtureAggregate),
    );

    expect(generation.selection.translation?.endpoint).toMatchObject({
      kind: "custom",
      apiBaseUrl: "https://example.com/v1",
    });
    expect(generation.credentials.length).toBeGreaterThan(0);
    expect(presentationText).not.toContain("https://");
    expect(presentationText).not.toContain("displaySuffix");
    expect(presentationText).not.toContain("credentials");
  });

  test("exposes only the safe endpoint kind for Official Translation", () => {
    const custom = translationGeneration("bilingual");
    if (custom.selection.translation === null) {
      throw new Error("Translation fixture requires a selection.");
    }
    const official: RuntimeGenerationSnapshot = {
      ...custom,
      selection: {
        ...custom.selection,
        translation: {
          ...custom.selection.translation,
          endpoint: { kind: "official" },
        },
      },
    };

    expect(translationPresentation(official, fixtureAggregate)).toMatchObject({
      endpointKind: "official",
    });
  });

  test("does not cross a stopped, stale, or replacement generation", () => {
    const oldGeneration = translationGeneration("bilingual");
    const replacementGeneration = translationGeneration(
      "bilingual",
      undefined,
      8,
    );
    const staleStream: CaptionAggregateSnapshot = {
      ...fixtureAggregate,
      activeStream: { generation: 7, streamId: "recognition-7-2" },
    };

    expect(translationPresentation(oldGeneration, null).units).toEqual([]);
    expect(
      translationPresentation(replacementGeneration, fixtureAggregate).units,
    ).toEqual([]);
    expect(translationPresentation(oldGeneration, staleStream).units).toEqual(
      [],
    );
  });

  test("shows only the restarted generation after a fresh aggregate pull", () => {
    const replacementGeneration = translationGeneration(
      "bilingual",
      undefined,
      8,
    );
    const restartedAggregate = aggregateInScope(
      fixtureAggregate,
      8,
      "recognition-8-1",
    );
    const presentation = translationPresentation(
      replacementGeneration,
      restartedAggregate,
    );

    expect(presentation.units).toHaveLength(3);
    expect(
      presentation.units.every(
        (unit) =>
          unit.sourceRef.generation === 8 &&
          unit.sourceRef.streamId === "recognition-8-1",
      ),
    ).toBe(true);
  });
});

import { describe, expect, test } from "vitest";
import captionAggregateFixture from "../../../contracts/caption-aggregate-snapshot-v2.json?raw";
import {
  CaptionAggregateContractError,
  decodeCaptionAggregateSnapshot,
} from "./captionAggregateContract";

type DecodedAggregate = ReturnType<typeof decodeCaptionAggregateSnapshot>;
type DecodedSourceRef = NonNullable<
  DecodedAggregate["captions"][number]["sourceRef"]
>;

describe("caption aggregate contract", () => {
  test("decodes the shared Caption Aggregate fixture", () => {
    const decoded = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );

    expect(decoded).toEqual({
      contractVersion: 2,
      snapshotRevision: 9,
      activeStream: { generation: 7, streamId: "recognition-7-1" },
      openSourceUnits: [],
      captions: [
        {
          generation: 7,
          streamId: "recognition-7-1",
          unitId: "speech-7-3",
          lane: "source",
          revision: 1,
          text: "Source whose translation failed.",
          state: "completed",
          language: "en",
          sourceRef: null,
          unitStartedAtMs: 1400,
          timestampMs: 1600,
        },
        {
          generation: 7,
          streamId: "recognition-7-1",
          unitId: "speech-7-2",
          lane: "source",
          revision: 1,
          text: "Source awaiting translation.",
          state: "completed",
          language: "en",
          sourceRef: null,
          unitStartedAtMs: 1200,
          timestampMs: 1400,
        },
        {
          generation: 7,
          streamId: "recognition-7-1",
          unitId: "speech-7-1",
          lane: "source",
          revision: 1,
          text: "Source with a completed translation.",
          state: "completed",
          language: "en",
          sourceRef: null,
          unitStartedAtMs: 1000,
          timestampMs: 1200,
        },
        {
          generation: 7,
          streamId: "recognition-7-1",
          unitId: "speech-7-1",
          lane: "translation",
          revision: 1,
          text: "已完成翻译的原文。",
          state: "completed",
          language: "zh-Hans",
          sourceRef: {
            generation: 7,
            streamId: "recognition-7-1",
            unitId: "speech-7-1",
            revision: 1,
          },
          unitStartedAtMs: 1000,
          timestampMs: 1201,
        },
      ],
      translationUnits: [
        {
          state: "failed",
          sourceRef: {
            generation: 7,
            streamId: "recognition-7-1",
            unitId: "speech-7-3",
            revision: 1,
          },
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
        },
        {
          state: "completed",
          sourceRef: {
            generation: 7,
            streamId: "recognition-7-1",
            unitId: "speech-7-1",
            revision: 1,
          },
        },
      ],
    });
  });

  test("rejects the old Caption Aggregate contract version", () => {
    expect(() =>
      decodeCaptionAggregateSnapshot({
        ...(JSON.parse(captionAggregateFixture) as object),
        contractVersion: 1,
      }),
    ).toThrow(
      "Invalid caption aggregate payload at $.contractVersion: expected 2.",
    );
  });

  test("rejects an ongoing Translation caption for a completed outcome", () => {
    const fixture = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );
    const translation = fixture.captions.find(
      (caption) => caption.lane === "translation",
    );

    expect(translation).toBeDefined();
    expect(() =>
      decodeCaptionAggregateSnapshot({
        ...fixture,
        captions: fixture.captions.map((caption) =>
          caption.lane === "translation"
            ? { ...caption, state: "ongoing" }
            : caption,
        ),
      }),
    ).toThrow(/only completed translation units/u);
  });

  test.each([
    [
      "source metadata left in the application contract",
      (fixture: ReturnType<typeof decodeCaptionAggregateSnapshot>) => ({
        ...fixture,
        captions: fixture.captions.map((caption, index) =>
          index === 0 ? { ...caption, provider: "openai" } : caption,
        ),
      }),
      /\.captions\[0\]\.provider/u,
    ],
    [
      "a source caption linked to another source",
      (fixture: ReturnType<typeof decodeCaptionAggregateSnapshot>) => ({
        ...fixture,
        captions: fixture.captions.map((caption, index) =>
          index === 0
            ? {
                ...caption,
                sourceRef: fixture.captions.find(
                  (candidate) => candidate.lane === "translation",
                )?.sourceRef,
              }
            : caption,
        ),
      }),
      /sourceRef/u,
    ],
  ] as const)("rejects %s", (_name, mutate, path) => {
    const fixture = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );

    expect(() => decodeCaptionAggregateSnapshot(mutate(fixture))).toThrow(path);
  });

  test.each([
    [
      "an orphan open source unit",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        activeStream: null,
        openSourceUnits: [{ unitId: "speech-orphan", startedAtMs: 1300 }],
      }),
      /openSourceUnits/u,
    ],
    [
      "duplicate open source unit identities",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        openSourceUnits: [
          { unitId: "speech-duplicate", startedAtMs: 1300 },
          { unitId: "speech-duplicate", startedAtMs: 1400 },
        ],
      }),
      /openSourceUnits/u,
    ],
    [
      "an ongoing caption in the wrong active stream",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        captions: fixture.captions.map((caption) =>
          caption.lane === "source"
            ? {
                ...caption,
                streamId: "recognition-7-stale",
                state: "ongoing",
              }
            : caption,
        ),
      }),
      /active caption stream/u,
    ],
    [
      "an ongoing source without an open unit",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        captions: fixture.captions.map((caption) =>
          caption.lane === "source"
            ? { ...caption, state: "ongoing" }
            : caption,
        ),
      }),
      /openSourceUnits/u,
    ],
    [
      "a completed source whose unit remains open",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        openSourceUnits: [{ unitId: "speech-7-1", startedAtMs: 1000 }],
      }),
      /completed source caption units cannot remain open/u,
    ],
    [
      "a duplicate lane correlation scope",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        captions: fixture.captions.flatMap((caption) =>
          caption.lane === "source"
            ? [caption, { ...caption, revision: caption.revision + 1 }]
            : [caption],
        ),
      }),
      /caption lane correlation scopes must be unique/u,
    ],
  ] as const)("rejects %s", (_name, mutate, expectation) => {
    const fixture = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );

    expect(() => decodeCaptionAggregateSnapshot(mutate(fixture))).toThrow(
      expectation,
    );
  });

  test.each([
    [
      "generation",
      (sourceRef: DecodedSourceRef) => ({
        ...sourceRef,
        generation: sourceRef.generation + 1,
      }),
    ],
    [
      "stream",
      (sourceRef: DecodedSourceRef) => ({
        ...sourceRef,
        streamId: `${sourceRef.streamId}-stale`,
      }),
    ],
    [
      "unit",
      (sourceRef: DecodedSourceRef) => ({
        ...sourceRef,
        unitId: `${sourceRef.unitId}-stale`,
      }),
    ],
    [
      "revision",
      (sourceRef: DecodedSourceRef) => ({
        ...sourceRef,
        revision: sourceRef.revision + 1,
      }),
    ],
  ] as const)(
    "rejects a translation sourceRef with a mismatched %s",
    (_dimension, mutateSourceRef) => {
      const fixture = decodeCaptionAggregateSnapshot(
        JSON.parse(captionAggregateFixture) as unknown,
      );

      expect(() =>
        decodeCaptionAggregateSnapshot({
          ...fixture,
          captions: fixture.captions.map((caption) =>
            caption.lane === "translation" && caption.sourceRef !== null
              ? {
                  ...caption,
                  sourceRef: mutateSourceRef(caption.sourceRef),
                }
              : caption,
          ),
        }),
      ).toThrow(/sourceRef/u);
    },
  );

  test("rejects duplicate translation outcomes for the same exact Source snapshot", () => {
    const fixture = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );
    const failed = fixture.translationUnits.find(
      (translationUnit) => translationUnit.state === "failed",
    );
    const pending = fixture.translationUnits.find(
      (translationUnit) => translationUnit.state === "pending",
    );
    expect(failed).toBeDefined();
    expect(pending).toBeDefined();

    expect(() =>
      decodeCaptionAggregateSnapshot({
        ...fixture,
        translationUnits: fixture.translationUnits.map((translationUnit) =>
          translationUnit === failed && pending !== undefined
            ? { ...translationUnit, sourceRef: pending.sourceRef }
            : translationUnit,
        ),
      }),
    ).toThrow(/source references must be unique/u);
  });

  test.each([
    [
      "generation",
      (sourceRef: DecodedSourceRef) => ({
        ...sourceRef,
        generation: sourceRef.generation + 1,
      }),
    ],
    [
      "stream",
      (sourceRef: DecodedSourceRef) => ({
        ...sourceRef,
        streamId: `${sourceRef.streamId}-stale`,
      }),
    ],
    [
      "unit",
      (sourceRef: DecodedSourceRef) => ({
        ...sourceRef,
        unitId: `${sourceRef.unitId}-stale`,
      }),
    ],
    [
      "revision",
      (sourceRef: DecodedSourceRef) => ({
        ...sourceRef,
        revision: sourceRef.revision + 1,
      }),
    ],
  ] as const)(
    "rejects a translation outcome sourceRef with a mismatched %s",
    (_dimension, mutateSourceRef) => {
      const fixture = decodeCaptionAggregateSnapshot(
        JSON.parse(captionAggregateFixture) as unknown,
      );

      expect(() =>
        decodeCaptionAggregateSnapshot({
          ...fixture,
          translationUnits: fixture.translationUnits.map((translationUnit) =>
            translationUnit.state === "failed"
              ? {
                  ...translationUnit,
                  sourceRef: mutateSourceRef(translationUnit.sourceRef),
                }
              : translationUnit,
          ),
        }),
      ).toThrow(/translationUnits.sourceRef/u);
    },
  );

  test("rejects a pending translation outcome outside the active caption stream", () => {
    const fixture = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );

    expect(() =>
      decodeCaptionAggregateSnapshot({
        ...fixture,
        activeStream: null,
      }),
    ).toThrow(
      /pending translation units must belong to the active caption stream/u,
    );
  });

  test.each([
    [
      "a completed outcome without its Translation caption",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        captions: fixture.captions.filter(
          (caption) => caption.lane !== "translation",
        ),
      }),
      /only completed translation units/u,
    ],
    [
      "a failed outcome carrying a Translation caption",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        translationUnits: fixture.translationUnits.map((translationUnit) =>
          translationUnit.state === "completed"
            ? {
                state: "failed",
                sourceRef: translationUnit.sourceRef,
                reasonCode: "translation.failed",
              }
            : translationUnit,
        ),
      }),
      /only completed translation units/u,
    ],
    [
      "a Translation caption without its completed outcome",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        translationUnits: fixture.translationUnits.filter(
          (translationUnit) => translationUnit.state !== "completed",
        ),
      }),
      /Translation captions require one exact completed translation unit/u,
    ],
  ] as const)("rejects %s", (_name, mutate, expectation) => {
    const fixture = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );

    expect(() => decodeCaptionAggregateSnapshot(mutate(fixture))).toThrow(
      expectation,
    );
  });

  test.each([
    [
      "an unknown translation failure reason",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        translationUnits: fixture.translationUnits.map((translationUnit) =>
          translationUnit.state === "failed"
            ? { ...translationUnit, reasonCode: "translation.provider_message" }
            : translationUnit,
        ),
      }),
      /reasonCode/u,
    ],
    [
      "a reason on a pending outcome",
      (fixture: DecodedAggregate) => ({
        ...fixture,
        translationUnits: fixture.translationUnits.map((translationUnit) =>
          translationUnit.state === "pending"
            ? { ...translationUnit, reasonCode: "translation.failed" }
            : translationUnit,
        ),
      }),
      /reasonCode/u,
    ],
  ] as const)("rejects %s", (_name, mutate, path) => {
    const fixture = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );

    expect(() => decodeCaptionAggregateSnapshot(mutate(fixture))).toThrow(path);
  });

  test("preserves the aggregate contract error type", () => {
    expect(() => decodeCaptionAggregateSnapshot(null)).toThrow(
      CaptionAggregateContractError,
    );
  });
});

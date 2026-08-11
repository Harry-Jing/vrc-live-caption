import { describe, expect, test } from "vitest";
import captionAggregateFixture from "../../../contracts/caption-aggregate-snapshot-v1.json?raw";
import {
  CaptionAggregateContractError,
  decodeCaptionAggregateSnapshot,
} from "./captionAggregateContract";

describe("caption aggregate contract", () => {
  test("decodes the shared Caption Aggregate fixture", () => {
    const decoded = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );

    expect(decoded).toEqual({
      contractVersion: 1,
      snapshotRevision: 4,
      activeStream: { generation: 7, streamId: "recognition-7-1" },
      openSourceUnits: [],
      captions: [
        {
          generation: 7,
          streamId: "recognition-7-1",
          unitId: "speech-7-1",
          lane: "source",
          revision: 1,
          text: "Full bounded OpenAI transcript.",
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
          text: "完整的有界转写。",
          state: "completed",
          language: "zh",
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
    });
  });

  test("rejects the old Caption Aggregate contract version", () => {
    expect(() =>
      decodeCaptionAggregateSnapshot({
        ...(JSON.parse(captionAggregateFixture) as object),
        contractVersion: 2,
      }),
    ).toThrow(
      "Invalid caption aggregate payload at $.contractVersion: expected 1.",
    );
  });

  test("admits an ongoing translation after its source lane completed", () => {
    const fixture = decodeCaptionAggregateSnapshot(
      JSON.parse(captionAggregateFixture) as unknown,
    );
    const source = fixture.captions[0];

    expect(source?.lane).toBe("source");
    expect(() =>
      decodeCaptionAggregateSnapshot({
        ...fixture,
        captions: [
          source,
          {
            ...fixture.captions[1],
            state: "ongoing",
            revision: 2,
            text: "仍在翻译",
          },
        ],
      }),
    ).not.toThrow();
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
            ? { ...caption, sourceRef: fixture.captions[1]?.sourceRef }
            : caption,
        ),
      }),
      /sourceRef/u,
    ],
    [
      "a translation linked to a stale source revision",
      (fixture: ReturnType<typeof decodeCaptionAggregateSnapshot>) => ({
        ...fixture,
        captions: fixture.captions.map((caption, index) =>
          index === 1 && caption.sourceRef !== null
            ? {
                ...caption,
                sourceRef: { ...caption.sourceRef, revision: 2 },
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

  test("preserves the aggregate contract error type", () => {
    expect(() => decodeCaptionAggregateSnapshot(null)).toThrow(
      CaptionAggregateContractError,
    );
  });
});

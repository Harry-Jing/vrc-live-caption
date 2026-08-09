import { describe, expect, test } from "vitest";
import captionSessionFixture from "../../contracts/caption-session-snapshot-v1.json?raw";
import {
  CaptionContractError,
  decodeCaptionSessionSnapshotV1,
} from "./captionSessionContract";

describe("caption session contract", () => {
  test("decodes the shared Rust and TypeScript V1 fixture", () => {
    const decoded = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );

    expect(decoded).toEqual({
      contractVersion: 1,
      snapshotRevision: 3,
      active: {
        generation: 7,
        streamId: "recognition-7-1",
      },
      activeUnits: [],
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
          provider: "openai",
          model: "gpt-transcribe",
          unitStartedAtMs: 1000,
          timestampMs: 1200,
        },
      ],
    });
  });

  test("rejects the removed stable caption state", () => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    const completed = valid.captions[0];

    expect(completed).toBeDefined();
    expect(() =>
      decodeCaptionSessionSnapshotV1({
        ...valid,
        captions: [{ ...completed, state: "stable" }],
      }),
    ).toThrow(/\.captions\[0\]\.state/);
  });

  test.each([
    {
      scope: "top-level snapshot",
      payload: (valid: ReturnType<typeof decodeCaptionSessionSnapshotV1>) => ({
        ...valid,
        stable: true,
      }),
      path: /\$\.stable/,
    },
    {
      scope: "active session",
      payload: (valid: ReturnType<typeof decodeCaptionSessionSnapshotV1>) => ({
        ...valid,
        active: { ...valid.active, stable: true },
      }),
      path: /\$\.active\.stable/,
    },
    {
      scope: "active unit",
      payload: (valid: ReturnType<typeof decodeCaptionSessionSnapshotV1>) => ({
        ...valid,
        activeUnits: [
          { unitId: "speech-extra", startedAtMs: 1300, stable: true },
        ],
      }),
      path: /\$\.activeUnits\[0\]\.stable/,
    },
    {
      scope: "caption",
      payload: (valid: ReturnType<typeof decodeCaptionSessionSnapshotV1>) => ({
        ...valid,
        captions: valid.captions.map((caption) => ({
          ...caption,
          stable: true,
        })),
      }),
      path: /\$\.captions\[0\]\.stable/,
    },
  ])("rejects unknown fields on a $scope object", ({ payload, path }) => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );

    expect(() => decodeCaptionSessionSnapshotV1(payload(valid))).toThrow(path);
  });

  test("rejects active caption units outside an active session", () => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );

    expect(() =>
      decodeCaptionSessionSnapshotV1({
        ...valid,
        active: null,
        activeUnits: [{ unitId: "speech-orphan", startedAtMs: 1300 }],
      }),
    ).toThrow(/activeUnits/);
  });

  test("rejects duplicate caption-unit identities", () => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );

    expect(() =>
      decodeCaptionSessionSnapshotV1({
        ...valid,
        activeUnits: [
          { unitId: "speech-duplicate", startedAtMs: 1300 },
          { unitId: "speech-duplicate", startedAtMs: 1400 },
        ],
      }),
    ).toThrow(/activeUnits/);
  });

  test("rejects an ongoing caption outside the active stream", () => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    const completed = valid.captions[0];

    expect(completed).toBeDefined();
    expect(() =>
      decodeCaptionSessionSnapshotV1({
        ...valid,
        captions: [
          {
            ...completed,
            streamId: "recognition-7-stale",
            state: "ongoing",
          },
        ],
      }),
    ).toThrow(/active/);
  });

  test("rejects a unitful ongoing caption for an unregistered unit", () => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    const completed = valid.captions[0];

    expect(completed).toBeDefined();
    expect(() =>
      decodeCaptionSessionSnapshotV1({
        ...valid,
        captions: [{ ...completed, state: "ongoing" }],
      }),
    ).toThrow(/activeUnits/);
  });

  test("rejects a completed caption whose unit is still active", () => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );

    expect(() =>
      decodeCaptionSessionSnapshotV1({
        ...valid,
        activeUnits: [{ unitId: "speech-7-1", startedAtMs: 1000 }],
      }),
    ).toThrow(/completed/);
  });

  test("allows completed history to reuse a unit identity in another stream", () => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );

    expect(() =>
      decodeCaptionSessionSnapshotV1({
        ...valid,
        active: { generation: 8, streamId: "recognition-8-1" },
        activeUnits: [{ unitId: "speech-7-1", startedAtMs: 2000 }],
      }),
    ).not.toThrow();
  });

  test("rejects duplicate lane snapshots for one correlation scope", () => {
    const valid = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    const completed = valid.captions[0];

    expect(completed).toBeDefined();
    expect(() =>
      decodeCaptionSessionSnapshotV1({
        ...valid,
        captions: [completed, { ...completed, revision: 2 }],
      }),
    ).toThrow(/unique/);
  });

  test("preserves the caption contract error type", () => {
    expect(() => decodeCaptionSessionSnapshotV1(null)).toThrow(
      CaptionContractError,
    );
  });
});

import { describe, expect, test } from "vitest";
import captionSessionFixture from "../../contracts/caption-session-snapshot-v1.json?raw";
import {
  createCaptionSessionState,
  decodeCaptionSessionSnapshotV1,
  reduceCaptionSessionState,
  selectCaptionSessionView,
} from "./captionSession";

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

  test("keeps a newer pushed aggregate when an older reload pull arrives", () => {
    const pulled = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    const pushed = {
      ...pulled,
      snapshotRevision: pulled.snapshotRevision + 1,
      captions: pulled.captions.map((caption) => ({
        ...caption,
        text: "Newer full caption.",
      })),
    };
    let state = createCaptionSessionState();

    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: pushed,
    });
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: pulled,
    });

    expect(state.snapshot?.snapshotRevision).toBe(4);
    expect(state.snapshot?.captions[0]?.text).toBe("Newer full caption.");
  });

  test("closes caption admission immediately on local Stop", () => {
    const active = decodeCaptionSessionSnapshotV1({
      contractVersion: 1,
      snapshotRevision: 10,
      active: { generation: 8, streamId: "recognition-8-1" },
      activeUnits: [{ unitId: "speech-8-1", startedAtMs: 2000 }],
      captions: [
        {
          generation: 8,
          streamId: "recognition-8-1",
          unitId: "speech-8-1",
          lane: "source",
          revision: 1,
          text: "Ongoing before Stop",
          state: "ongoing",
          language: "en",
          provider: "mock",
          model: "mock",
          unitStartedAtMs: 2000,
          timestampMs: 2100,
        },
      ],
    });
    let state = createCaptionSessionState();
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: active,
    });
    state = reduceCaptionSessionState(state, { type: "stopRequested" });
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: {
        ...active,
        snapshotRevision: 11,
        captions: active.captions.map((caption) => ({
          ...caption,
          revision: 2,
          text: "Late after Stop",
        })),
      },
    });

    expect(state.snapshot?.snapshotRevision).toBe(10);
    expect(state.snapshot?.captions[0]?.text).toBe("Ongoing before Stop");
  });

  test("keeps the latest completion visible when Stop hides ongoing text", () => {
    const fixture = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    const completed = fixture.captions[0];

    expect(completed).toBeDefined();
    const active = decodeCaptionSessionSnapshotV1({
      ...fixture,
      snapshotRevision: 20,
      activeUnits: [{ unitId: "speech-7-2", startedAtMs: 1300 }],
      captions: [
        {
          ...completed,
          unitId: "speech-7-2",
          state: "ongoing",
          text: "Ongoing text",
          unitStartedAtMs: 1300,
          timestampMs: 1400,
        },
        completed,
      ],
    });
    let state = createCaptionSessionState();
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: active,
    });

    expect(selectCaptionSessionView(state, true).captionMode).toBe("partial");

    state = reduceCaptionSessionState(state, { type: "stopRequested" });
    const stopped = selectCaptionSessionView(state, true);

    expect(stopped.captionMode).toBe("final");
    expect(stopped.visibleCaption?.text).toBe(
      "Full bounded OpenAI transcript.",
    );
    expect(stopped.completedCaptions).toHaveLength(1);
  });

  test("shows listening while partial text is hidden and preserves the latest completion", () => {
    const fixture = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    const completed = fixture.captions[0];

    expect(completed).toBeDefined();
    const snapshot = decodeCaptionSessionSnapshotV1({
      ...fixture,
      snapshotRevision: 21,
      activeUnits: [{ unitId: "speech-7-2", startedAtMs: 1300 }],
      captions: [
        {
          ...completed,
          unitId: "speech-7-2",
          state: "ongoing",
          text: "Hidden ongoing text",
          unitStartedAtMs: 1300,
          timestampMs: 1400,
        },
        completed,
      ],
    });
    const state = reduceCaptionSessionState(createCaptionSessionState(), {
      type: "snapshotReceived",
      snapshot,
    });
    const view = selectCaptionSessionView(state, false);

    expect(view.captionMode).toBe("listening");
    expect(view.visibleCaption?.text).toBe("Full bounded OpenAI transcript.");
  });

  test("projects the backend-ordered five most recent source completions", () => {
    const fixture = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    const completed = fixture.captions[0];

    expect(completed).toBeDefined();
    const snapshot = decodeCaptionSessionSnapshotV1({
      ...fixture,
      snapshotRevision: 22,
      captions: Array.from({ length: 6 }, (_, index) => ({
        ...completed,
        unitId: `speech-history-${String(6 - index)}`,
        text: `Completion ${String(6 - index)}`,
        unitStartedAtMs: 2000 + index,
        timestampMs: 2100 + index,
      })),
    });
    const state = reduceCaptionSessionState(createCaptionSessionState(), {
      type: "snapshotReceived",
      snapshot,
    });
    const view = selectCaptionSessionView(state, true);

    expect(view.completedCaptions.map((caption) => caption.text)).toEqual([
      "Completion 6",
      "Completion 5",
      "Completion 4",
      "Completion 3",
      "Completion 2",
    ]);
    expect(view.visibleCaption?.text).toBe("Completion 6");
  });

  test("reopens only for a backend-issued generation newer than the stopped run", () => {
    const fixture = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    let state = createCaptionSessionState();
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: fixture,
    });
    state = reduceCaptionSessionState(state, { type: "stopRequested" });
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: {
        ...fixture,
        snapshotRevision: 4,
        active: null,
        activeUnits: [],
      },
    });
    state = reduceCaptionSessionState(state, { type: "startSucceeded" });
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: { ...fixture, snapshotRevision: 5 },
    });

    expect(state.snapshot?.active).toBeNull();
    expect(state.admission).toBe("awaitingStartSnapshot");

    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: {
        ...fixture,
        snapshotRevision: 6,
        active: { generation: 8, streamId: "recognition-8-1" },
        captions: [],
      },
    });

    expect(state.snapshot?.active).toEqual({
      generation: 8,
      streamId: "recognition-8-1",
    });
    expect(state.admission).toBe("open");
  });

  test("stays open when the new backend snapshot arrives before Start returns", () => {
    const started = decodeCaptionSessionSnapshotV1(
      JSON.parse(captionSessionFixture) as unknown,
    );
    let state = createCaptionSessionState();
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: started,
    });
    state = reduceCaptionSessionState(state, { type: "startSucceeded" });
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: started,
    });

    expect(state.admission).toBe("open");
    expect(state.snapshot).toBe(started);
  });

  test("reopens the authoritative running session when Stop fails", () => {
    const active = decodeCaptionSessionSnapshotV1({
      contractVersion: 1,
      snapshotRevision: 30,
      active: { generation: 9, streamId: "recognition-9-1" },
      activeUnits: [{ unitId: "speech-9-1", startedAtMs: 3000 }],
      captions: [
        {
          generation: 9,
          streamId: "recognition-9-1",
          unitId: "speech-9-1",
          lane: "source",
          revision: 1,
          text: "Still running",
          state: "ongoing",
          language: "en",
          provider: "mock",
          model: "mock",
          unitStartedAtMs: 3000,
          timestampMs: 3100,
        },
      ],
    });
    let state = createCaptionSessionState();
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: active,
    });
    state = reduceCaptionSessionState(state, { type: "stopRequested" });
    state = reduceCaptionSessionState(state, { type: "stopFailed" });
    state = reduceCaptionSessionState(state, {
      type: "snapshotReceived",
      snapshot: { ...active, snapshotRevision: 31 },
    });

    expect(state.admission).toBe("open");
    expect(selectCaptionSessionView(state, true)).toMatchObject({
      captionMode: "partial",
      visibleCaption: { text: "Still running" },
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
});

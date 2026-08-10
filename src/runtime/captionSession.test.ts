import { describe, expect, test } from "vitest";
import captionSessionFixture from "../../contracts/caption-session-snapshot-v1.json?raw";
import {
  createCaptionSessionState,
  reduceCaptionSessionState,
  selectCaptionSessionView,
} from "./captionSession";
import { decodeCaptionSessionSnapshotV1 } from "./wire/captionSessionContract";

describe("caption session state", () => {
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

    expect(selectCaptionSessionView(state, true).captionPreviewStatus).toBe(
      "ongoing",
    );

    state = reduceCaptionSessionState(state, { type: "stopRequested" });
    const stopped = selectCaptionSessionView(state, true);

    expect(stopped.captionPreviewStatus).toBe("completed");
    expect(stopped.visibleCaption?.text).toBe(
      "Full bounded OpenAI transcript.",
    );
    expect(stopped.completedCaptions).toHaveLength(1);
  });

  test("shows listening while ongoing text is hidden and preserves the latest completion", () => {
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

    expect(view.captionPreviewStatus).toBe("listening");
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
      captionPreviewStatus: "ongoing",
      visibleCaption: { text: "Still running" },
    });
  });
});

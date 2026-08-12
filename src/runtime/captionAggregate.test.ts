import { describe, expect, test } from "vitest";
import captionAggregateFixture from "../../contracts/caption-aggregate-snapshot-v2.json?raw";
import {
  createCaptionAggregateState,
  reduceCaptionAggregateState,
  selectCaptionAggregateView,
} from "./captionAggregate";
import { decodeCaptionAggregateSnapshot } from "./wire/captionAggregateContract";

function fixture() {
  const decoded = decodeCaptionAggregateSnapshot(
    JSON.parse(captionAggregateFixture) as unknown,
  );
  const source = decoded.captions.find(
    (caption) => caption.lane === "source" && caption.unitId === "speech-7-1",
  );
  if (source === undefined) {
    throw new Error(
      "The Caption Aggregate fixture must contain its first Source unit.",
    );
  }

  return decodeCaptionAggregateSnapshot({
    ...decoded,
    snapshotRevision: 4,
    captions: [{ ...source, text: "Full bounded OpenAI transcript." }],
    translationUnits: [],
  });
}

describe("caption aggregate state", () => {
  test("keeps a newer pushed aggregate when an older reload pull arrives", () => {
    const pulled = fixture();
    const pushed = {
      ...pulled,
      snapshotRevision: pulled.snapshotRevision + 1,
      captions: pulled.captions.map((caption) => ({
        ...caption,
        text: "Newer full caption.",
      })),
    };
    let state = createCaptionAggregateState();

    state = reduceCaptionAggregateState(state, {
      type: "snapshotReceived",
      snapshot: pushed,
    });
    state = reduceCaptionAggregateState(state, {
      type: "snapshotReceived",
      snapshot: pulled,
    });

    expect(state.snapshot?.snapshotRevision).toBe(5);
    expect(state.snapshot?.captions[0]?.text).toBe("Newer full caption.");
  });

  test("closes caption admission immediately on local Stop", () => {
    const source = fixture().captions[0];
    expect(source).toBeDefined();
    const active = decodeCaptionAggregateSnapshot({
      contractVersion: 2,
      snapshotRevision: 10,
      activeStream: { generation: 8, streamId: "recognition-8-1" },
      openSourceUnits: [{ unitId: "speech-8-1", startedAtMs: 2000 }],
      captions: [
        {
          ...source,
          generation: 8,
          streamId: "recognition-8-1",
          unitId: "speech-8-1",
          state: "ongoing",
          text: "Ongoing before Stop",
          sourceRef: null,
          unitStartedAtMs: 2000,
          timestampMs: 2100,
        },
      ],
      translationUnits: [],
    });
    let state = reduceCaptionAggregateState(createCaptionAggregateState(), {
      type: "snapshotReceived",
      snapshot: active,
    });
    state = reduceCaptionAggregateState(state, { type: "stopRequested" });
    state = reduceCaptionAggregateState(state, {
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

  test("keeps the latest source completion visible when Stop hides ongoing text", () => {
    const aggregate = fixture();
    const completed = aggregate.captions[0];
    expect(completed).toBeDefined();
    const active = decodeCaptionAggregateSnapshot({
      ...aggregate,
      snapshotRevision: 20,
      openSourceUnits: [{ unitId: "speech-7-2", startedAtMs: 1300 }],
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
    let state = reduceCaptionAggregateState(createCaptionAggregateState(), {
      type: "snapshotReceived",
      snapshot: active,
    });

    expect(selectCaptionAggregateView(state, true).captionPreviewStatus).toBe(
      "ongoing",
    );

    state = reduceCaptionAggregateState(state, { type: "stopRequested" });
    const stopped = selectCaptionAggregateView(state, true);

    expect(stopped.captionPreviewStatus).toBe("completed");
    expect(stopped.visibleCaption?.text).toBe(
      "Full bounded OpenAI transcript.",
    );
    expect(stopped.completedCaptions).toHaveLength(1);
  });

  test("shows listening while ongoing text is hidden", () => {
    const aggregate = fixture();
    const completed = aggregate.captions[0];
    expect(completed).toBeDefined();
    const snapshot = decodeCaptionAggregateSnapshot({
      ...aggregate,
      snapshotRevision: 21,
      openSourceUnits: [{ unitId: "speech-7-2", startedAtMs: 1300 }],
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
    const state = reduceCaptionAggregateState(createCaptionAggregateState(), {
      type: "snapshotReceived",
      snapshot,
    });
    const view = selectCaptionAggregateView(state, false);

    expect(view.captionPreviewStatus).toBe("listening");
    expect(view.visibleCaption?.text).toBe("Full bounded OpenAI transcript.");
  });

  test("projects the application-ordered five most recent source completions", () => {
    const aggregate = fixture();
    const completed = aggregate.captions[0];
    expect(completed).toBeDefined();
    const snapshot = decodeCaptionAggregateSnapshot({
      ...aggregate,
      snapshotRevision: 22,
      captions: Array.from({ length: 6 }, (_, index) => ({
        ...completed,
        unitId: `speech-history-${String(6 - index)}`,
        text: `Completion ${String(6 - index)}`,
        unitStartedAtMs: 2000 + index,
        timestampMs: 2100 + index,
      })),
    });
    const state = reduceCaptionAggregateState(createCaptionAggregateState(), {
      type: "snapshotReceived",
      snapshot,
    });
    const view = selectCaptionAggregateView(state, true);

    expect(view.completedCaptions.map((caption) => caption.text)).toEqual([
      "Completion 6",
      "Completion 5",
      "Completion 4",
      "Completion 3",
      "Completion 2",
    ]);
  });

  test("reopens only for an application-issued generation newer than the stopped run", () => {
    const aggregate = fixture();
    let state = reduceCaptionAggregateState(createCaptionAggregateState(), {
      type: "snapshotReceived",
      snapshot: aggregate,
    });
    state = reduceCaptionAggregateState(state, { type: "stopRequested" });
    state = reduceCaptionAggregateState(state, {
      type: "snapshotReceived",
      snapshot: {
        ...aggregate,
        snapshotRevision: 5,
        activeStream: null,
        openSourceUnits: [],
      },
    });
    state = reduceCaptionAggregateState(state, { type: "startSucceeded" });
    state = reduceCaptionAggregateState(state, {
      type: "snapshotReceived",
      snapshot: { ...aggregate, snapshotRevision: 6 },
    });

    expect(state.snapshot?.activeStream).toBeNull();
    expect(state.admission).toBe("awaitingStartSnapshot");

    state = reduceCaptionAggregateState(state, {
      type: "snapshotReceived",
      snapshot: {
        ...aggregate,
        snapshotRevision: 7,
        activeStream: { generation: 8, streamId: "recognition-8-1" },
        captions: [],
      },
    });

    expect(state.snapshot?.activeStream?.generation).toBe(8);
    expect(state.admission).toBe("open");
  });

  test("reopens the authoritative running aggregate when Stop fails", () => {
    const aggregate = fixture();
    let state = reduceCaptionAggregateState(createCaptionAggregateState(), {
      type: "snapshotReceived",
      snapshot: aggregate,
    });
    state = reduceCaptionAggregateState(state, { type: "stopRequested" });
    state = reduceCaptionAggregateState(state, { type: "stopFailed" });

    expect(state.admission).toBe("open");
  });
});

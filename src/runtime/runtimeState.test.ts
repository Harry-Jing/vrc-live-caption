import { describe, expect, test } from "vitest";
import {
  createRuntimeState,
  reduceRuntimeState,
  selectRuntimeView,
} from "./runtimeState";
import type { RuntimeState, RuntimeStateInput } from "./runtimeState";
import type {
  DiagnosticEvent,
  RuntimeStatusEvent,
  RuntimeEvent,
  TranscriptEvent,
  UtteranceEndedEvent,
  UtteranceStartedEvent,
} from "./types";

type RuntimeCommandRequestedInput = Omit<
  Extract<RuntimeStateInput, { type: "runtimeCommandRequested" }>,
  "attemptId"
>;
type RuntimeStateTestInput = RuntimeStateInput | RuntimeCommandRequestedInput;

const initialStatus: RuntimeStatusEvent = {
  status: "idle",
  message: "Runtime is idle",
  timestampMs: 0,
};

function status(
  value: RuntimeStatusEvent["status"],
  timestampMs: number,
): RuntimeStatusEvent {
  return {
    status: value,
    message: value,
    timestampMs,
  };
}

function diagnostic(id: string, timestampMs: number): DiagnosticEvent {
  return {
    id,
    category: "runtime",
    severity: "warning",
    code: "runtime.test",
    message: id,
    timestampMs,
  };
}

let nextTestCommandAttemptId = 0;

function apply(state: RuntimeState, input: RuntimeStateTestInput) {
  if (input.type === "runtimeCommandRequested" && !("attemptId" in input)) {
    nextTestCommandAttemptId += 1;

    return reduceRuntimeState(state, {
      ...input,
      attemptId: nextTestCommandAttemptId,
    });
  }

  return reduceRuntimeState(state, input);
}

function backendEvent(event: RuntimeEvent): RuntimeStateInput {
  return { type: "backendEvent", event };
}

function started(
  utteranceId: string,
  timestampMs: number,
): UtteranceStartedEvent {
  return {
    id: `started-${utteranceId}`,
    utteranceId,
    timestampMs,
  };
}

function ended(utteranceId: string, timestampMs: number): UtteranceEndedEvent {
  return {
    id: `ended-${utteranceId}`,
    utteranceId,
    reason: "noSpeech",
    timestampMs,
  };
}

function finalTranscript(
  utteranceId: string,
  revision: number,
  text: string,
  timestampMs: number,
): TranscriptEvent {
  return {
    id: `final-${utteranceId}-${String(revision)}`,
    utteranceId,
    kind: "final",
    text,
    language: "en",
    provider: "mock",
    revision,
    timestampMs,
  };
}

function partialTranscript(
  utteranceId: string,
  revision: number,
  text: string,
  timestampMs: number,
): TranscriptEvent {
  return {
    ...finalTranscript(utteranceId, revision, text, timestampMs),
    id: `partial-${utteranceId}-${String(revision)}-${text}`,
    kind: "partial",
  };
}

describe("runtime state", () => {
  test("presents an empty session as waiting instead of a completed caption", () => {
    const view = selectRuntimeView(createRuntimeState(initialStatus), {
      showPartial: true,
    });

    expect(view.captionMode).toBe("waiting");
    expect(view.visibleTranscript).toBeNull();
  });

  test("keeps the previous completed caption visible while the next unit is listening", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 20) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("a", 1, "Previous caption", 30),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("b", 40) }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("listening");
    expect(view.visibleTranscript?.text).toBe("Previous caption");
    expect(view.finalTranscripts.map((event) => event.text)).toEqual([
      "Previous caption",
    ]);
  });

  test("ignores duplicate and older revisions instead of moving an ongoing caption backwards", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 20) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 2, "newest text", 30),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 2, "conflicting duplicate", 40),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 1, "older text", 50),
      }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("partial");
    expect(view.visibleTranscript?.text).toBe("newest text");
  });

  test("keeps a completed unit terminal when a later partial arrives", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 20) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 1, "ongoing", 30),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("a", 2, "completed", 40),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 3, "late partial", 50),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 60) }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("final");
    expect(view.visibleTranscript?.text).toBe("completed");
    expect(view.finalTranscripts).toHaveLength(1);
  });

  test("rejects stopped and known previous-generation events after a quick restart", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("old", 20) }),
    );
    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "stop_runtime",
      timestampMs: 30,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("old", 1, "late while stopped", 40),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("stopped", 45) }),
    );
    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 50,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("old", 1, "old generation", 60),
      }),
    );

    expect(
      selectRuntimeView(state, { showPartial: true }).visibleTranscript,
    ).toBeNull();

    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 70) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("new", 1, "current generation", 80),
      }),
    );

    expect(
      selectRuntimeView(state, { showPartial: true }).visibleTranscript?.text,
    ).toBe("current generation");
  });

  test("does not let an older reload snapshot overwrite a status event received during reload", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 200) }),
    );
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("stopped", 150),
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("running");
  });

  test("replays caption events that arrive while a running reload snapshot is in flight", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("during-reload", 1, "kept", 30),
      }),
    );
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("running", 20),
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).visibleTranscript?.text,
    ).toBe("kept");
  });

  test("does not reopen or replay a reload buffer after Stop is requested", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("before-stop", 1, "must be discarded", 30),
      }),
    );
    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "stop_runtime",
      timestampMs: 40,
    });
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("running", 20),
    });

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.runtimeStatus.status).toBe("stopping");
    expect(view.captionMode).toBe("waiting");
    expect(view.visibleTranscript).toBeNull();
  });

  test("does not let an older unit ending clear a newer active unit", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("old", 20) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 30) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("new", 1, "newer unit", 40),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceEnded", payload: ended("old", 50) }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("partial");
    expect(view.visibleTranscript?.text).toBe("newer unit");
    expect(view.finalTranscripts).toHaveLength(0);
  });

  test("treats a repeated older unit start as a no-op", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("old", 20) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 30) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("old", 40) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceEnded", payload: ended("old", 50) }),
    );

    expect(selectRuntimeView(state, { showPartial: false }).captionMode).toBe(
      "listening",
    );
  });

  test("does not let an older unit partial replace the newer active unit", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("old", 20) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 30) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("new", 1, "newer partial", 40),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("old", 1, "late older partial", 50),
      }),
    );

    expect(
      selectRuntimeView(state, { showPartial: true }).visibleTranscript?.text,
    ).toBe("newer partial");
  });

  test("does not assign a new meaning to the unused stable transcript kind", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 20) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: {
          ...partialTranscript("a", 1, "not a partial", 30),
          kind: "stable",
        },
      }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("listening");
    expect(view.visibleTranscript).toBeNull();
  });

  test("deduplicates diagnostic events without applying the caption generation gate", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(
      state,
      backendEvent({ type: "diagnostic", payload: diagnostic("same", 10) }),
    );
    state = apply(
      state,
      backendEvent({ type: "diagnostic", payload: diagnostic("same", 10) }),
    );

    expect(selectRuntimeView(state, { showPartial: true }).diagnostics).toEqual(
      [diagnostic("same", 10)],
    );
  });

  test("does not let a stale terminal status from the previous run close a restarted generation", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 20) }),
    );
    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "stop_runtime",
      timestampMs: 30,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("stopped", 45) }),
    );
    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 50,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("stopped", 40) }),
    );
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 60) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 70) }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.runtimeStatus.status).toBe("running");
    expect(view.captionMode).toBe("listening");
  });

  test("does not let a late older unit start steal listening from a newer unit", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 40) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("old", 30) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceEnded", payload: ended("old", 50) }),
    );

    expect(selectRuntimeView(state, { showPartial: false }).captionMode).toBe(
      "listening",
    );
  });

  test("does not resurrect an older partial after a newer unit completed", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("old", 20) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 30) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("new", 1, "new completed", 40),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("old", 1, "late old partial", 50),
      }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("final");
    expect(view.visibleTranscript?.text).toBe("new completed");
  });

  test("orders completed history by event time instead of late arrival order", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });

    for (const event of [
      finalTranscript("first", 1, "first", 30),
      finalTranscript("newest", 1, "newest", 50),
      finalTranscript("middle-late", 1, "middle", 40),
    ]) {
      state = apply(
        state,
        backendEvent({ type: "transcriptFinal", payload: event }),
      );
    }

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.visibleTranscript?.text).toBe("newest");
    expect(view.finalTranscripts.map((event) => event.text)).toEqual([
      "newest",
      "middle",
      "first",
    ]);
  });

  test("keeps the previous completion visible when partial preview is disabled", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("done", 1, "previous", 20),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("next", 30) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("next", 1, "hidden partial", 40),
      }),
    );

    const view = selectRuntimeView(state, { showPartial: false });

    expect(view.captionMode).toBe("listening");
    expect(view.visibleTranscript?.text).toBe("previous");
  });

  test("drops buffered reload events when synchronization is cancelled", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("buffered", 1, "discard me", 20),
      }),
    );
    state = apply(state, {
      type: "runtimeStatusSyncCancelled",
      requestId: 1,
    });
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("running", 10),
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).visibleTranscript,
    ).toBeNull();
  });

  test("closes a speculative generation when Start fails", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("speculative", 1, "discard me", 15),
      }),
    );
    state = apply(state, {
      type: "runtimeCommandFailed",
      attemptId: nextTestCommandAttemptId,
      command: "start_runtime",
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("late", 20) }),
    );

    expect(selectRuntimeView(state, { showPartial: true }).captionMode).toBe(
      "waiting",
    );
    expect(
      selectRuntimeView(state, { showPartial: true }).finalTranscripts,
    ).toHaveLength(0);
  });

  test("allows a recovery snapshot older than a failed Start intent", () => {
    let state = createRuntimeState(status("idle", 100));

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 200,
    });
    state = apply(state, {
      type: "runtimeCommandFailed",
      attemptId: 1,
      command: "start_runtime",
    });
    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("running", 150),
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("live", 160) }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.runtimeStatus.status).toBe("running");
    expect(view.captionMode).toBe("listening");
  });

  test("rejects an unseen event timestamped before the restarted generation fence", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 100,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("never-seen-old", 1, "old", 99),
      }),
    );

    expect(
      selectRuntimeView(state, { showPartial: true }).visibleTranscript,
    ).toBeNull();
  });

  test("clears an older partial when a newer utterance starts", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("old", 1, "old partial", 20),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 30) }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("listening");
    expect(view.visibleTranscript).toBeNull();
  });

  test("bounds completed history, diagnostics, and utterance tombstones", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 1,
    });

    for (let index = 1; index <= 300; index += 1) {
      state = apply(
        state,
        backendEvent({
          type: "transcriptFinal",
          payload: finalTranscript(
            `unit-${String(index)}`,
            1,
            `caption-${String(index)}`,
            index + 1,
          ),
        }),
      );
    }

    for (let index = 1; index <= 60; index += 1) {
      state = apply(
        state,
        backendEvent({
          type: "diagnostic",
          payload: diagnostic(`diagnostic-${String(index)}`, 400 + index),
        }),
      );
    }

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.finalTranscripts).toHaveLength(5);
    expect(view.diagnostics).toHaveLength(50);
    expect(state.trackedUtterances).toHaveLength(256);
  });

  test("uses a late Started event to correct unit order without freezing later partials", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 1,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 1, "a revision 1", 100),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 10) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 2, "a revision 2", 110),
      }),
    );

    expect(
      selectRuntimeView(state, { showPartial: true }).visibleTranscript?.text,
    ).toBe("a revision 2");

    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("b", 50) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("b", 1, "newer unit", 60),
      }),
    );

    expect(
      selectRuntimeView(state, { showPartial: true }).visibleTranscript?.text,
    ).toBe("newer unit");
  });

  test("orders late completed events by caption unit start instead of completion time", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 1,
    });
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 20) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("b", 30) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("b", 1, "newer unit", 40),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("a", 1, "late older unit", 50),
      }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.visibleTranscript?.text).toBe("newer unit");
    expect(view.finalTranscripts.map((event) => event.text)).toEqual([
      "newer unit",
      "late older unit",
    ]);
  });

  test("treats the synthetic initial status as non-authoritative during reload", () => {
    let state = createRuntimeState(status("idle", 100));

    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 90) }),
    );
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("running", 90),
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("running");
  });

  test("ignores a failed duplicate Start attempt without closing the first generation", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 10,
    });

    expect(state.runtimeStatus.status).toBe("starting");

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 2,
      command: "start_runtime",
      timestampMs: 11,
    });
    state = apply(state, {
      type: "runtimeCommandFailed",
      attemptId: 2,
      command: "start_runtime",
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 12) }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("new", 13) }),
    );

    expect(selectRuntimeView(state, { showPartial: true }).captionMode).toBe(
      "listening",
    );
  });

  test("restores the correct partial when late Started events repair unit order", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 1,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("b", 1, "B remains active", 90),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("b", 20) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 1, "A was provisionally newer", 100),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 10) }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("partial");
    expect(view.visibleTranscript?.utteranceId).toBe("b");
    expect(view.visibleTranscript?.text).toBe("B remains active");
  });

  test("restores an older active partial when a terminal unit is reordered behind it", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 1,
    });
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("b", 1, "B is still ongoing", 90),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("a", 1, "A looked newer", 100),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("a", 2, "A completed", 110),
      }),
    );
    state = apply(
      state,
      backendEvent({ type: "utteranceStarted", payload: started("a", 10) }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.captionMode).toBe("partial");
    expect(view.visibleTranscript?.utteranceId).toBe("b");
    expect(view.visibleTranscript?.text).toBe("B is still ongoing");
  });

  test("retains the newest active ordering anchor when the ledger is overloaded", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      command: "start_runtime",
      timestampMs: 1,
    });
    state = apply(
      state,
      backendEvent({
        type: "utteranceStarted",
        payload: started("anchor", 1_000),
      }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptPartial",
        payload: partialTranscript("anchor", 1, "newest active unit", 1_001),
      }),
    );

    for (let index = 1; index <= 300; index += 1) {
      state = apply(
        state,
        backendEvent({
          type: "utteranceStarted",
          payload: started(`late-old-${String(index)}`, index),
        }),
      );
    }

    const view = selectRuntimeView(state, { showPartial: true });

    expect(state.trackedUtterances).toHaveLength(256);
    expect(view.captionMode).toBe("partial");
    expect(view.visibleTranscript?.utteranceId).toBe("anchor");
    expect(view.visibleTranscript?.text).toBe("newest active unit");
  });

  test("lets a newer running snapshot reopen a backend-inactive reload fence", () => {
    let state = createRuntimeState(status("idle", 100));

    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("stopped", 80) }),
    );
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("running", 90),
    });
    state = apply(
      state,
      backendEvent({
        type: "utteranceStarted",
        payload: started("after-reload", 95),
      }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.runtimeStatus.status).toBe("running");
    expect(view.captionMode).toBe("listening");
  });

  test("keeps a local Stop fence closed even if a newer running status arrives", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 20) }),
    );
    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 2,
      command: "stop_runtime",
      timestampMs: 30,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 40) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "transcriptFinal",
        payload: finalTranscript("late", 1, "must stay hidden", 50),
      }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.runtimeStatus.status).toBe("running");
    expect(view.captionMode).toBe("waiting");
    expect(view.visibleTranscript).toBeNull();
  });

  test("gives a new Start intent precedence over an equal-time old stopped status", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 300,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("stopped", 300) }),
    );
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 301) }),
    );
    state = apply(
      state,
      backendEvent({
        type: "utteranceStarted",
        payload: started("new-generation", 302),
      }),
    );

    const view = selectRuntimeView(state, { showPartial: true });

    expect(view.runtimeStatus.status).toBe("running");
    expect(view.captionMode).toBe("listening");
  });

  test("accepts a genuine equal-time startup error from a later pull", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 300,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("stopped", 300) }),
    );
    state = apply(state, {
      type: "runtimeCommandSucceeded",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 301,
    });
    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("error", 300),
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("error");
  });

  test("converges to stopped from a successful command when status pushes are missed", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 20) }),
    );
    state = apply(state, {
      type: "runtimeCommandSucceeded",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 21,
    });
    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 2,
      command: "stop_runtime",
      timestampMs: 30,
    });
    state = apply(state, {
      type: "runtimeCommandSucceeded",
      attemptId: 2,
      command: "stop_runtime",
      timestampMs: 40,
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("stopped");
  });

  test("does not let a late stopping push roll back a successful Stop acknowledgement", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 20) }),
    );
    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 2,
      command: "stop_runtime",
      timestampMs: 30,
    });
    state = apply(state, {
      type: "runtimeCommandSucceeded",
      attemptId: 2,
      command: "stop_runtime",
      timestampMs: 40,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("stopping", 35) }),
    );
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 40) }),
    );

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("stopped");
  });

  test("does not discard newer status evidence when Stop fails", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(state, {
      type: "runtimeCommandSucceeded",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 20,
    });
    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 2,
      command: "stop_runtime",
      timestampMs: 30,
    });
    state = apply(
      state,
      backendEvent({ type: "status", payload: status("running", 40) }),
    );
    state = apply(state, {
      type: "runtimeCommandFailed",
      attemptId: 2,
      command: "stop_runtime",
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("running");
  });

  test("allows Stop to preempt a successful Start whose status pushes were missed", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 10,
    });
    state = apply(state, {
      type: "runtimeCommandSucceeded",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 20,
    });
    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 2,
      command: "stop_runtime",
      timestampMs: 30,
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("stopping");
  });

  test("converges when a later Start reconciliation observes running", () => {
    let state = createRuntimeState(initialStatus);

    state = apply(state, {
      type: "runtimeCommandRequested",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 100,
    });
    state = apply(state, {
      type: "runtimeCommandSucceeded",
      attemptId: 1,
      command: "start_runtime",
      timestampMs: 110,
    });
    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 1,
    });
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 1,
      snapshot: status("idle", 0),
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("starting");

    state = apply(state, {
      type: "runtimeStatusSyncStarted",
      requestId: 2,
    });
    state = apply(state, {
      type: "runtimeStatusSyncCompleted",
      requestId: 2,
      snapshot: status("running", 120),
    });

    expect(
      selectRuntimeView(state, { showPartial: true }).runtimeStatus.status,
    ).toBe("running");
  });
});

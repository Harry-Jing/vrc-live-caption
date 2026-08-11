import { describe, expect, test } from "vitest";
import {
  createRuntimeState,
  reduceRuntimeState,
  selectRuntimeView,
} from "./runtimeState";
import type { DiagnosticEvent, RuntimeStatusEvent } from "./runtimeEvents";

const idle: RuntimeStatusEvent = { status: "idle", timestampMs: 0 };

function status(
  value: RuntimeStatusEvent["status"],
  timestampMs: number,
): RuntimeStatusEvent {
  return { status: value, timestampMs };
}

function diagnostic(id: string): DiagnosticEvent {
  return {
    id,
    category: "runtime",
    severity: "warning",
    code: "runtime.test",
    message: id,
    timestampMs: 1,
  };
}

describe("runtime lifecycle state", () => {
  test("does not let an older reload pull overwrite a newer status push", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 2,
      snapshot: status("running", 20),
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 1,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 30) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCompleted",
      requestId: 1,
      controlRevision: 2,
      snapshot: status("running", 20),
    });

    expect(selectRuntimeView(state).runtimeStatus).toEqual(
      status("running", 30),
    );
  });

  test("accepts an authoritative control status after the wall clock moves backwards", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 4,
      snapshot: status("running", 1_000),
    });
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 5,
      snapshot: status("error", 900),
    });

    expect(state.runtimeStatus).toEqual(status("error", 900));

    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 950) },
    });

    expect(state.runtimeStatus).toEqual(status("error", 900));
  });

  test("accepts a newer control pull when a legacy status arrives during the pull", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 4,
      snapshot: status("running", 1_000),
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 1,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 1_100) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCompleted",
      requestId: 1,
      controlRevision: 5,
      snapshot: status("error", 900),
    });

    expect(state.runtimeStatus).toEqual(status("error", 900));
  });

  test("keeps an in-flight Start ahead of an inactive control snapshot", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 1_000,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 1,
      snapshot: status("stopped", 900),
    });

    expect(state.runtimeStatus).toEqual(status("starting", 1_000));
    expect(state.inFlightLifecycleAction?.action).toBe("start");
  });

  test.each(["starting", "running"] as const)(
    "keeps the Start attempt in flight while control reports %s",
    (controlStatus) => {
      let state = createRuntimeState(idle);
      state = reduceRuntimeState(state, {
        type: "runtimeActionRequested",
        attemptId: 1,
        action: "start",
        timestampMs: 1_000,
      });
      state = reduceRuntimeState(state, {
        type: "runtimeControlStatusReceived",
        revision: 1,
        snapshot: status(controlStatus, 900),
      });

      expect(state.runtimeStatus).toEqual(status(controlStatus, 900));
      expect(state.inFlightLifecycleAction?.attemptId).toBe(1);

      state = reduceRuntimeState(state, {
        type: "runtimeActionSucceeded",
        attemptId: 1,
        action: "start",
        timestampMs: 1_010,
      });

      expect(state.inFlightLifecycleAction).toBeNull();
    },
  );

  test("keeps an authoritative Error when the in-flight Start action fails", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 1_000,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 1,
      snapshot: status("error", 900),
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionFailed",
      attemptId: 1,
      action: "start",
    });

    expect(state.runtimeStatus).toEqual(status("error", 900));
    expect(state.inFlightLifecycleAction).toBeNull();
  });

  test("keeps a successful Start ahead of an older control revision", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 1,
      snapshot: idle,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 1_000,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionSucceeded",
      attemptId: 1,
      action: "start",
      timestampMs: 1_010,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 1,
      snapshot: status("idle", 900),
    });

    expect(state.runtimeStatus).toEqual(status("starting", 1_000));
  });

  test("keeps an in-flight Stop ahead of an active control snapshot", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 1,
      snapshot: status("running", 800),
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "stop",
      timestampMs: 1_000,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 2,
      snapshot: status("running", 900),
    });

    expect(state.runtimeStatus).toEqual(status("stopping", 1_000));
    expect(state.inFlightLifecycleAction?.action).toBe("stop");
  });

  test("lets a newer control revision supersede an acknowledged Stop", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 4,
      snapshot: status("running", 800),
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "stop",
      timestampMs: 1_000,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionSucceeded",
      attemptId: 1,
      action: "stop",
      timestampMs: 1_010,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 4,
      snapshot: status("running", 850),
    });

    expect(state.runtimeStatus).toEqual(status("stopped", 1_010));

    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 5,
      snapshot: status("running", 900),
    });

    expect(state.runtimeStatus).toEqual(status("running", 900));
  });

  test("restores the previous status when Start fails without newer evidence", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 10,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionFailed",
      attemptId: 1,
      action: "start",
    });

    expect(state.runtimeStatus).toBe(idle);
  });

  test("treats a successful Stop acknowledgement as authoritative", () => {
    let state = createRuntimeState(status("running", 10));
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "stop",
      timestampMs: 20,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionSucceeded",
      attemptId: 1,
      action: "stop",
      timestampMs: 30,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("stopping", 25) },
    });

    expect(state.runtimeStatus.status).toBe("stopped");
    expect(state.runtimeStatus.timestampMs).toBe(30);
  });

  test("keeps newer authoritative runtime evidence when Stop reports failure", () => {
    let state = createRuntimeState(status("running", 10));
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "stop",
      timestampMs: 20,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("stopped", 30) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionFailed",
      attemptId: 1,
      action: "stop",
    });

    expect(state.runtimeStatus.status).toBe("stopped");
  });

  test("does not let a stale terminal status from the previous run close a restart", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 10,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 20) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 2,
      action: "stop",
      timestampMs: 30,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("stopped", 45) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 3,
      action: "start",
      timestampMs: 50,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("stopped", 40) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 60) },
    });

    expect(state.runtimeStatus.status).toBe("running");
  });

  test("treats the synthetic initial status as non-authoritative during reload", () => {
    let state = createRuntimeState(status("idle", 100));
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 1,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 90) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCompleted",
      requestId: 1,
      controlRevision: 1,
      snapshot: status("running", 90),
    });

    expect(state.runtimeStatus.status).toBe("running");
  });

  test("clears a matching cancelled runtime status synchronization", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 7,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCancelled",
      requestId: 7,
    });

    expect(state.inFlightRuntimeStateSynchronization).toBeNull();
  });

  test("keeps a newer runtime status synchronization after a stale cancellation", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 7,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 8,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCancelled",
      requestId: 7,
    });

    expect(state.inFlightRuntimeStateSynchronization).toEqual({ requestId: 8 });

    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCompleted",
      requestId: 8,
      controlRevision: 1,
      snapshot: status("running", 10),
    });
    expect(state.runtimeStatus).toEqual(status("running", 10));
    expect(state.inFlightRuntimeStateSynchronization).toBeNull();
  });

  test("ignores a failed duplicate Start attempt", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 10,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 2,
      action: "start",
      timestampMs: 11,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionFailed",
      attemptId: 2,
      action: "start",
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 12) },
    });

    expect(state.runtimeStatus.status).toBe("running");
  });

  test("gives a new Start intent precedence over an equal-time stopped status", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 300,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("stopped", 300) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 301) },
    });

    expect(state.runtimeStatus.status).toBe("running");
  });

  test("accepts a genuine equal-time startup error from a later pull", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 300,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionSucceeded",
      attemptId: 1,
      action: "start",
      timestampMs: 301,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 1,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCompleted",
      requestId: 1,
      controlRevision: 1,
      snapshot: status("error", 300),
    });

    expect(state.runtimeStatus.status).toBe("error");
  });

  test("does not let a late stopping push roll back a successful Stop", () => {
    let state = createRuntimeState(status("running", 20));
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "stop",
      timestampMs: 30,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionSucceeded",
      attemptId: 1,
      action: "stop",
      timestampMs: 40,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("stopping", 35) },
    });
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "status", payload: status("running", 40) },
    });

    expect(state.runtimeStatus.status).toBe("stopped");
  });

  test("allows Stop to preempt a successful Start whose pushes were missed", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 10,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionSucceeded",
      attemptId: 1,
      action: "start",
      timestampMs: 20,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 2,
      action: "stop",
      timestampMs: 30,
    });

    expect(state.runtimeStatus.status).toBe("stopping");
  });

  test("converges when a later Start reconciliation observes running", () => {
    let state = createRuntimeState(idle);
    state = reduceRuntimeState(state, {
      type: "runtimeControlStatusReceived",
      revision: 1,
      snapshot: idle,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionRequested",
      attemptId: 1,
      action: "start",
      timestampMs: 100,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeActionSucceeded",
      attemptId: 1,
      action: "start",
      timestampMs: 110,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 1,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCompleted",
      requestId: 1,
      controlRevision: 1,
      snapshot: status("idle", 0),
    });
    expect(state.runtimeStatus.status).toBe("starting");

    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationStarted",
      requestId: 2,
    });
    state = reduceRuntimeState(state, {
      type: "runtimeStateSynchronizationCompleted",
      requestId: 2,
      controlRevision: 2,
      snapshot: status("running", 120),
    });

    expect(state.runtimeStatus.status).toBe("running");
  });

  test("deduplicates and bounds diagnostics independently of captions", () => {
    let state = createRuntimeState(idle);

    for (let index = 0; index < 55; index += 1) {
      state = reduceRuntimeState(state, {
        type: "runtimeEventReceived",
        event: { type: "diagnostic", payload: diagnostic(String(index)) },
      });
    }
    state = reduceRuntimeState(state, {
      type: "runtimeEventReceived",
      event: { type: "diagnostic", payload: diagnostic("54") },
    });

    expect(state.diagnostics).toHaveLength(50);
    expect(state.diagnostics[0]?.id).toBe("54");
  });
});

import { describe, expect, test, vi } from "vitest";
import { normalizeAppFailure } from "./appFailure";
import { createRuntimeSynchronizationGate } from "./runtimeSynchronization";

const normalizeFailure = (cause: unknown) =>
  normalizeAppFailure(cause, "Synchronization failed.");

describe("runtime synchronization gate", () => {
  test("keeps bootstrap errors independent until a complete retry succeeds", async () => {
    const gate = createRuntimeSynchronizationGate(normalizeFailure);

    await expect(
      gate.ensureSynchronized(() =>
        Promise.reject(new Error("listener unavailable")),
      ),
    ).resolves.toBe(false);
    expect(gate.snapshot()).toMatchObject({
      isSynchronized: false,
      isSynchronizing: false,
      failure: { code: null, message: "listener unavailable" },
    });

    // An unrelated action has no access to this error scope and cannot clear it.
    await Promise.resolve();
    expect(gate.snapshot().failure?.message).toBe("listener unavailable");

    await expect(
      gate.ensureSynchronized(() => Promise.resolve()),
    ).resolves.toBe(true);
    expect(gate.snapshot()).toEqual({
      isSynchronized: true,
      isSynchronizing: false,
      failure: null,
    });
  });

  test("deduplicates concurrent registration and pull attempts", async () => {
    let finishAttempt: (() => void) | undefined;
    const attempt = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finishAttempt = resolve;
        }),
    );
    const gate = createRuntimeSynchronizationGate(normalizeFailure);

    const first = gate.ensureSynchronized(attempt);
    const second = gate.ensureSynchronized(attempt);

    expect(attempt).toHaveBeenCalledTimes(1);
    expect(gate.snapshot()).toMatchObject({
      isSynchronized: false,
      isSynchronizing: true,
    });

    finishAttempt?.();
    await expect(Promise.all([first, second])).resolves.toEqual([true, true]);
    expect(attempt).toHaveBeenCalledTimes(1);
    expect(gate.snapshot().isSynchronized).toBe(true);
  });

  test("does not run a gated command when readiness recovery fails", async () => {
    const command = vi.fn(() => Promise.resolve());
    const gate = createRuntimeSynchronizationGate(normalizeFailure);

    const isSynchronized = await gate.ensureSynchronized(() =>
      Promise.reject(new Error("not connected")),
    );
    if (isSynchronized) {
      await command();
    }

    expect(command).not.toHaveBeenCalled();
    expect(gate.snapshot().failure?.message).toBe("not connected");
  });

  test("accepts listener evidence without letting an older failed attempt regress it", async () => {
    let failAttempt: ((error: Error) => void) | undefined;
    const gate = createRuntimeSynchronizationGate(normalizeFailure);
    const attempt = gate.ensureSynchronized(
      () =>
        new Promise<void>((_resolve, reject) => {
          failAttempt = reject;
        }),
    );

    gate.markSynchronized();
    failAttempt?.(new Error("stale pull failure"));

    await expect(attempt).resolves.toBe(true);
    expect(gate.snapshot()).toEqual({
      isSynchronized: true,
      isSynchronizing: false,
      failure: null,
    });
  });

  test("retains a structured synchronization failure code", async () => {
    const gate = createRuntimeSynchronizationGate(normalizeFailure);

    await gate.ensureSynchronized(() =>
      Promise.reject(
        Object.assign(new Error("Control snapshot unavailable."), {
          code: "runtime.state_failed",
        }),
      ),
    );

    expect(gate.snapshot().failure).toEqual({
      code: "runtime.state_failed",
      message: "Control snapshot unavailable.",
    });
  });
});

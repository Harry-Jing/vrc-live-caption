import { describe, expect, test, vi } from "vitest";
import { createRuntimeReadinessGate } from "./runtimeReadiness";

describe("runtime readiness gate", () => {
  test("keeps bootstrap errors independent until a complete retry succeeds", async () => {
    const gate = createRuntimeReadinessGate((error) => String(error));

    await expect(
      gate.ensure(() => Promise.reject(new Error("listener unavailable"))),
    ).resolves.toBe(false);
    expect(gate.snapshot()).toMatchObject({
      ready: false,
      isBusy: false,
      error: "Error: listener unavailable",
    });

    // An unrelated action has no access to this error scope and cannot clear it.
    await Promise.resolve();
    expect(gate.snapshot().error).toBe("Error: listener unavailable");

    await expect(gate.ensure(() => Promise.resolve())).resolves.toBe(true);
    expect(gate.snapshot()).toEqual({
      ready: true,
      isBusy: false,
      error: "",
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
    const gate = createRuntimeReadinessGate((error) => String(error));

    const first = gate.ensure(attempt);
    const second = gate.ensure(attempt);

    expect(attempt).toHaveBeenCalledTimes(1);
    expect(gate.snapshot()).toMatchObject({ ready: false, isBusy: true });

    finishAttempt?.();
    await expect(Promise.all([first, second])).resolves.toEqual([true, true]);
    expect(attempt).toHaveBeenCalledTimes(1);
    expect(gate.snapshot().ready).toBe(true);
  });

  test("does not run a gated command when readiness recovery fails", async () => {
    const command = vi.fn(() => Promise.resolve());
    const gate = createRuntimeReadinessGate((error) => String(error));

    const ready = await gate.ensure(() =>
      Promise.reject(new Error("not connected")),
    );
    if (ready) {
      await command();
    }

    expect(command).not.toHaveBeenCalled();
    expect(gate.snapshot().error).toBe("Error: not connected");
  });

  test("accepts listener evidence without letting an older failed attempt regress it", async () => {
    let failAttempt: ((error: Error) => void) | undefined;
    const gate = createRuntimeReadinessGate((error) => String(error));
    const attempt = gate.ensure(
      () =>
        new Promise<void>((_resolve, reject) => {
          failAttempt = reject;
        }),
    );

    gate.markReady();
    failAttempt?.(new Error("stale pull failure"));

    await expect(attempt).resolves.toBe(true);
    expect(gate.snapshot()).toEqual({
      ready: true,
      isBusy: false,
      error: "",
    });
  });
});

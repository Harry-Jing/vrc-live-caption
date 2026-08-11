import { describe, expect, test, vi } from "vitest";
import { createLifecycleActionQueue } from "./lifecycleActionQueue";

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });

  return { promise, reject, resolve };
}

describe("lifecycle action queue", () => {
  test("invokes Stop immediately while an in-flight Start settles", async () => {
    const start = deferred();
    const calls: string[] = [];
    const invoke = vi.fn((action: "start" | "stop") => {
      calls.push(action);
      return action === "start" ? start.promise : Promise.resolve();
    });
    const run = createLifecycleActionQueue(invoke);

    const startResult = run("start");
    await Promise.resolve();
    const stopResult = run("stop");
    await Promise.resolve();

    expect(calls).toEqual(["start", "stop"]);

    start.resolve();
    await startResult;
    await stopResult;

    expect(calls).toEqual(["start", "stop"]);
  });

  test("does not let a failed Start prevent the preempting Stop", async () => {
    const start = deferred();
    const calls: string[] = [];
    const invoke = vi.fn((action: "start" | "stop") => {
      calls.push(action);
      return action === "start" ? start.promise : Promise.resolve();
    });
    const run = createLifecycleActionQueue(invoke);

    const startResult = run("start");
    await Promise.resolve();
    const stopResult = run("stop");
    const failure = new Error("Start failed");

    start.reject(failure);

    await expect(startResult).rejects.toBe(failure);
    await expect(stopResult).resolves.toBeUndefined();
    expect(calls).toEqual(["start", "stop"]);
  });

  test("drops a Start that was still queued when Stop preempted it", async () => {
    const firstStart = deferred();
    const calls: string[] = [];
    const invoke = vi.fn((action: "start" | "stop") => {
      calls.push(action);
      return action === "start" ? firstStart.promise : Promise.resolve();
    });
    const run = createLifecycleActionQueue(invoke);

    const firstStartResult = run("start");
    await Promise.resolve();
    const queuedStartResult = run("start");
    const stopResult = run("stop");
    await stopResult;

    expect(calls).toEqual(["start", "stop"]);

    firstStart.resolve();
    await firstStartResult;
    await queuedStartResult;
    expect(calls).toEqual(["start", "stop"]);
  });
});

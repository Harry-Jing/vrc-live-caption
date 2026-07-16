import { describe, expect, test, vi } from "vitest";
import { createLifecycleCommandQueue } from "./lifecycleCommandQueue";

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });

  return { promise, reject, resolve };
}

describe("lifecycle command queue", () => {
  test("runs a later Stop only after an in-flight Start settles", async () => {
    const start = deferred();
    const calls: string[] = [];
    const invoke = vi.fn((command: "start_runtime" | "stop_runtime") => {
      calls.push(command);
      return command === "start_runtime" ? start.promise : Promise.resolve();
    });
    const run = createLifecycleCommandQueue(invoke);

    const startResult = run("start_runtime");
    await Promise.resolve();
    const stopResult = run("stop_runtime");
    await Promise.resolve();

    expect(calls).toEqual(["start_runtime"]);

    start.resolve();
    await startResult;
    await stopResult;

    expect(calls).toEqual(["start_runtime", "stop_runtime"]);
  });

  test("does not let a failed Start prevent the queued Stop", async () => {
    const start = deferred();
    const calls: string[] = [];
    const invoke = vi.fn((command: "start_runtime" | "stop_runtime") => {
      calls.push(command);
      return command === "start_runtime" ? start.promise : Promise.resolve();
    });
    const run = createLifecycleCommandQueue(invoke);

    const startResult = run("start_runtime");
    await Promise.resolve();
    const stopResult = run("stop_runtime");
    const failure = new Error("Start failed");

    start.reject(failure);

    await expect(startResult).rejects.toBe(failure);
    await expect(stopResult).resolves.toBeUndefined();
    expect(calls).toEqual(["start_runtime", "stop_runtime"]);
  });
});

import type { RuntimeCommand } from "./types";

export type RuntimeLifecycleCommand = Extract<
  RuntimeCommand,
  "start_runtime" | "stop_runtime"
>;

export function createLifecycleCommandQueue(
  invoke: (command: RuntimeLifecycleCommand) => Promise<void>,
) {
  let tail: Promise<void> = Promise.resolve();

  return (command: RuntimeLifecycleCommand) => {
    const result = tail.then(() => invoke(command));

    // A failed command must reject its own caller without poisoning later
    // lifecycle work. In particular, a Stop queued behind a failed Start still
    // needs to reach the backend and confirm the runtime is inactive.
    tail = result.catch(() => undefined);

    return result;
  };
}

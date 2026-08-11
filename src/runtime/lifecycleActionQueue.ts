import type { RuntimeLifecycleAction } from "./lifecycle";

export function createLifecycleActionQueue(
  invoke: (action: RuntimeLifecycleAction) => Promise<void>,
) {
  let workTail: Promise<void> = Promise.resolve();
  let stopVersion = 0;

  return (action: RuntimeLifecycleAction) => {
    if (action === "stop") {
      // Stop is a hard trust boundary, so it must not queue behind a Start
      // that may be blocked in configuration or credential I/O. Future Starts
      // still wait until both the preempted work and this Stop have settled.
      stopVersion += 1;
      const result = Promise.resolve().then(() => invoke(action));
      workTail = Promise.all([workTail, result.catch(() => undefined)]).then(
        () => undefined,
      );

      return result;
    }

    const requestedAtStopVersion = stopVersion;
    const result = workTail.then(() => {
      // A Start that never reached the host runtime before a later Stop must
      // not be invoked afterward and accidentally create a post-Stop generation.
      if (requestedAtStopVersion !== stopVersion) {
        return;
      }

      return invoke(action);
    });

    // A failed action must reject its own caller without poisoning later
    // lifecycle work.
    workTail = result.catch(() => undefined);

    return result;
  };
}

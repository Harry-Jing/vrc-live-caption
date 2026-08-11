import type { AppFailure } from "./appFailure";

export type RuntimeSynchronizationSnapshot = Readonly<{
  isSynchronized: boolean;
  isSynchronizing: boolean;
  failure: AppFailure | null;
}>;

type NormalizeRuntimeSynchronizationFailure = (cause: unknown) => AppFailure;
type RuntimeSynchronizationListener = (
  snapshot: RuntimeSynchronizationSnapshot,
) => void;

const INITIAL_RUNTIME_SYNCHRONIZATION: RuntimeSynchronizationSnapshot = {
  isSynchronized: false,
  isSynchronizing: false,
  failure: null,
};

export function createRuntimeSynchronizationGate(
  normalizeFailure: NormalizeRuntimeSynchronizationFailure,
  listener: RuntimeSynchronizationListener = () => undefined,
) {
  let current = INITIAL_RUNTIME_SYNCHRONIZATION;
  let attemptInFlight: Promise<boolean> | null = null;

  function setSnapshot(snapshot: RuntimeSynchronizationSnapshot) {
    current = snapshot;
    listener(snapshot);
  }

  function markSynchronized() {
    if (
      current.isSynchronized &&
      !current.isSynchronizing &&
      current.failure === null
    ) {
      return;
    }

    setSnapshot({
      isSynchronized: true,
      isSynchronizing: false,
      failure: null,
    });
  }

  function ensureSynchronized(attempt: () => Promise<void>): Promise<boolean> {
    if (current.isSynchronized) {
      return Promise.resolve(true);
    }

    if (attemptInFlight !== null) {
      return attemptInFlight;
    }

    setSnapshot({ ...current, isSynchronizing: true });

    let attemptResult: Promise<void>;

    try {
      attemptResult = attempt();
    } catch (cause) {
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- Adapter failures are unknown until the gate's AppFailure normalizer handles them.
      attemptResult = Promise.reject(cause);
    }

    const attemptPromise = attemptResult
      .then(
        () => {
          markSynchronized();
          return true;
        },
        (cause: unknown) => {
          if (current.isSynchronized) {
            return true;
          }

          setSnapshot({
            isSynchronized: false,
            isSynchronizing: false,
            failure: normalizeFailure(cause),
          });
          return false;
        },
      )
      .finally(() => {
        if (attemptInFlight === attemptPromise) {
          attemptInFlight = null;
        }
      });

    attemptInFlight = attemptPromise;
    return attemptPromise;
  }

  return {
    ensureSynchronized,
    markSynchronized,
    snapshot: () => current,
  };
}

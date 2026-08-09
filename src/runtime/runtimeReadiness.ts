export type RuntimeReadinessSnapshot = Readonly<{
  ready: boolean;
  isBusy: boolean;
  error: string;
}>;

type NormalizeRuntimeReadinessError = (error: unknown) => string;
type RuntimeReadinessListener = (snapshot: RuntimeReadinessSnapshot) => void;

const INITIAL_RUNTIME_READINESS: RuntimeReadinessSnapshot = {
  ready: false,
  isBusy: false,
  error: "",
};

export function createRuntimeReadinessGate(
  normalizeError: NormalizeRuntimeReadinessError,
  listener: RuntimeReadinessListener = () => undefined,
) {
  let current = INITIAL_RUNTIME_READINESS;
  let attemptInFlight: Promise<boolean> | null = null;

  function setSnapshot(snapshot: RuntimeReadinessSnapshot) {
    current = snapshot;
    listener(snapshot);
  }

  function markReady() {
    if (current.ready && !current.isBusy && current.error === "") {
      return;
    }

    setSnapshot({ ready: true, isBusy: false, error: "" });
  }

  function ensure(attempt: () => Promise<void>): Promise<boolean> {
    if (current.ready) {
      return Promise.resolve(true);
    }

    if (attemptInFlight !== null) {
      return attemptInFlight;
    }

    setSnapshot({ ...current, isBusy: true });

    let attemptResult: Promise<void>;

    try {
      attemptResult = attempt();
    } catch (error) {
      attemptResult = Promise.reject(
        error instanceof Error ? error : new Error(normalizeError(error)),
      );
    }

    const attemptPromise = attemptResult
      .then(
        () => {
          markReady();
          return true;
        },
        (error: unknown) => {
          if (current.ready) {
            return true;
          }

          setSnapshot({
            ready: false,
            isBusy: false,
            error: normalizeError(error),
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
    ensure,
    markReady,
    snapshot: () => current,
  };
}

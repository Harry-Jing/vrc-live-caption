import type { RuntimeSessionPhase, RuntimeStatus } from "./types";

const ACTIVE_SESSION_PHASES: ReadonlySet<RuntimeSessionPhase> = new Set([
  "starting",
  "running",
  "reconnecting",
  "stopping",
]);

const ACTIVE_RUNTIME_STATUSES: ReadonlySet<RuntimeStatus> = new Set([
  "starting",
  "running",
  "reconnecting",
  "stopping",
]);

const STOPPABLE_RUNTIME_STATUSES: ReadonlySet<RuntimeStatus> = new Set([
  "starting",
  "running",
  "reconnecting",
  "error",
]);

export function isActiveRuntimeSessionPhase(
  phase: RuntimeSessionPhase | null | undefined,
) {
  return (
    phase !== null && phase !== undefined && ACTIVE_SESSION_PHASES.has(phase)
  );
}

export function isActiveRuntimeStatus(status: RuntimeStatus) {
  return ACTIVE_RUNTIME_STATUSES.has(status);
}

export function isStoppableRuntimeStatus(status: RuntimeStatus) {
  return STOPPABLE_RUNTIME_STATUSES.has(status);
}

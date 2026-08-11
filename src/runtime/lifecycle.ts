import type { RuntimeGenerationPhase } from "./runtimeControl";
import type { RuntimeStatus } from "./runtimeEvents";

export type RuntimeAction = "start" | "stop" | "testChatbox";
export type RuntimeLifecycleAction = Extract<RuntimeAction, "start" | "stop">;

const ACTIVE_GENERATION_PHASES: ReadonlySet<RuntimeGenerationPhase> = new Set([
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

export function isActiveRuntimeGenerationPhase(
  phase: RuntimeGenerationPhase | null | undefined,
) {
  return (
    phase !== null && phase !== undefined && ACTIVE_GENERATION_PHASES.has(phase)
  );
}

export function isActiveRuntimeStatus(status: RuntimeStatus) {
  return ACTIVE_RUNTIME_STATUSES.has(status);
}

export function isStoppableRuntimeStatus(status: RuntimeStatus) {
  return STOPPABLE_RUNTIME_STATUSES.has(status);
}

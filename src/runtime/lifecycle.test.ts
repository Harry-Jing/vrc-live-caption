import { describe, expect, it } from "vitest";
import {
  isActiveRuntimeGenerationPhase,
  isActiveRuntimeStatus,
  isStoppableRuntimeStatus,
} from "./lifecycle";
import type { RuntimeGenerationPhase } from "./runtimeControl";
import type { RuntimeStatus } from "./runtimeEvents";

describe("runtime lifecycle predicates", () => {
  it("keeps active generation phases in one typed policy", () => {
    const phases: RuntimeGenerationPhase[] = [
      "starting",
      "running",
      "reconnecting",
      "stopping",
      "error",
    ];

    expect(phases.filter(isActiveRuntimeGenerationPhase)).toEqual([
      "starting",
      "running",
      "reconnecting",
      "stopping",
    ]);
    expect(isActiveRuntimeGenerationPhase(null)).toBe(false);
  });

  it("distinguishes active from stoppable runtime statuses", () => {
    const statuses: RuntimeStatus[] = [
      "idle",
      "starting",
      "running",
      "reconnecting",
      "stopping",
      "stopped",
      "error",
    ];

    expect(statuses.filter(isActiveRuntimeStatus)).toEqual([
      "starting",
      "running",
      "reconnecting",
      "stopping",
    ]);
    expect(statuses.filter(isStoppableRuntimeStatus)).toEqual([
      "starting",
      "running",
      "reconnecting",
      "error",
    ]);
  });
});

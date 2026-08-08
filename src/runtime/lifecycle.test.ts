import { describe, expect, it } from "vitest";
import {
  isActiveRuntimeSessionPhase,
  isActiveRuntimeStatus,
  isStoppableRuntimeStatus,
} from "./lifecycle";
import type { RuntimeSessionPhase, RuntimeStatus } from "./types";

describe("runtime lifecycle predicates", () => {
  it("keeps active session phases in one typed policy", () => {
    const phases: RuntimeSessionPhase[] = [
      "starting",
      "running",
      "reconnecting",
      "stopping",
      "error",
    ];

    expect(phases.filter(isActiveRuntimeSessionPhase)).toEqual([
      "starting",
      "running",
      "reconnecting",
      "stopping",
    ]);
    expect(isActiveRuntimeSessionPhase(null)).toBe(false);
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

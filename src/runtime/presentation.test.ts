import { describe, expect, test } from "vitest";
import {
  publicationDisplayPlanView,
  publicationPlanView,
  publicationSettingsView,
  publicationStartIsBlocked,
} from "./presentation";
import type { PublicationPlan, RuntimePlan } from "./types";

const recognition: RuntimePlan["recognition"] = {
  path: "openAiBounded",
  inputShape: "completedAudioUnits",
  boundaryOwner: "application",
  unitBehavior: "unitBased",
  lanes: [
    {
      lane: "source",
      updates: "completedOnly",
      revisions: "appendOnly",
    },
  ],
};

function runtimePlan(publication: PublicationPlan): RuntimePlan {
  return { recognition, publication };
}

const incompatibleLivePlan = runtimePlan({
  state: "incompatible",
  requestedMode: "live",
  selectedLanes: ["source"],
  reason: { reason: "modeUnsupported", lanes: ["source"] },
  supportedModes: ["completed"],
});

const completedPlan = runtimePlan({
  state: "ready",
  mode: "completed",
  policy: { policy: "completed" },
  selectedLanes: ["source"],
});

describe("publication plan presentation", () => {
  test("does not present a saved desired plan as validation for a dirty form", () => {
    expect(publicationSettingsView(incompatibleLivePlan, true)).toEqual({
      state: "unverified",
    });
  });

  test("preserves the backend-resolved policy and delay", () => {
    const unitPlan = runtimePlan({
      state: "ready",
      mode: "live",
      policy: { policy: "liveUnit", observationWindowMs: 750 },
      selectedLanes: ["source"],
    });
    const unitlessPlan = runtimePlan({
      state: "ready",
      mode: "live",
      policy: { policy: "liveUnitless", firstNonEmptyDelayMs: 1250 },
      selectedLanes: ["source"],
    });

    expect(publicationPlanView(unitPlan)).toEqual({
      state: "ready",
      mode: "live",
      policy: "liveUnit",
      delayMs: 750,
    });
    expect(publicationPlanView(unitlessPlan)).toEqual({
      state: "ready",
      mode: "live",
      policy: "liveUnitless",
      delayMs: 1250,
    });
  });

  test("keeps an incompatible request explicit without choosing a fallback", () => {
    expect(publicationPlanView(incompatibleLivePlan)).toEqual({
      state: "incompatible",
      mode: "live",
      supportedModes: ["completed"],
    });
  });

  test("blocks only a new Start for an incompatible desired plan", () => {
    expect(publicationStartIsBlocked(false, incompatibleLivePlan)).toBe(true);
    expect(publicationStartIsBlocked(true, incompatibleLivePlan)).toBe(false);
    expect(publicationStartIsBlocked(false, null)).toBe(false);
  });

  test("shows the active plan instead of a different next-Start plan", () => {
    expect(
      publicationDisplayPlanView(completedPlan, incompatibleLivePlan),
    ).toEqual({
      state: "ready",
      mode: "completed",
      policy: "completed",
      delayMs: null,
    });
    expect(publicationDisplayPlanView(null, incompatibleLivePlan)).toEqual({
      state: "incompatible",
      mode: "live",
      supportedModes: ["completed"],
    });
  });
});

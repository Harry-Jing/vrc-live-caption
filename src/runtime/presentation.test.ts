import { describe, expect, test } from "vitest";
import {
  publicationDisplayPlanView,
  publicationPlanView,
  publicationSettingsView,
  publicationStartIsBlocked,
} from "./presentation";
import type { CaptionPipelinePlan, PublicationPlan } from "./captionPipeline";

const recognition: CaptionPipelinePlan["recognition"] = {
  path: "openai/gpt-transcribe",
  inputShape: "continuousAudioFrames",
  captionBoundaryOwner: "application",
  unitBehavior: "unitBased",
  lanes: [
    {
      lane: "source",
      updates: "completedOnly",
      revisions: "appendOnly",
    },
  ],
};

function captionPipelinePlan(
  publication: PublicationPlan,
): CaptionPipelinePlan {
  return { recognition, publication };
}

const incompatibleLivePlan = captionPipelinePlan({
  state: "incompatible",
  requestedMode: "live",
  selectedLanes: ["source"],
  reason: { reason: "modeUnsupported", lanes: ["source"] },
  supportedModes: ["completed"],
});

const completedPlan = captionPipelinePlan({
  state: "compatible",
  mode: "completed",
  timing: { timing: "completed" },
  selectedLanes: ["source"],
});

describe("publication plan presentation", () => {
  test("does not present a saved desired plan as validation for a dirty form", () => {
    expect(publicationSettingsView(incompatibleLivePlan, true)).toEqual({
      state: "unverified",
    });
  });

  test("preserves the application-resolved unit policy and delay", () => {
    const unitPlan = captionPipelinePlan({
      state: "compatible",
      mode: "live",
      timing: { timing: "liveUnit", observationWindowMs: 750 },
      selectedLanes: ["source"],
    });

    expect(publicationPlanView(unitPlan)).toEqual({
      state: "compatible",
      mode: "live",
      timing: "liveUnit",
      delayMs: 750,
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
      state: "compatible",
      mode: "completed",
      timing: "completed",
      delayMs: null,
    });
    expect(publicationDisplayPlanView(null, incompatibleLivePlan)).toEqual({
      state: "incompatible",
      mode: "live",
      supportedModes: ["completed"],
    });
  });
});

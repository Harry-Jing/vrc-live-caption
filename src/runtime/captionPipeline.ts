import type { CaptionLane } from "./captionAggregate";

export const PUBLICATION_MODES = ["completed", "live"] as const;
export type PublicationMode = (typeof PUBLICATION_MODES)[number];

export const CONTENT_SELECTIONS = [
  "sourceOnly",
  "translationOnly",
  "bilingual",
] as const;
export type ContentSelection = (typeof CONTENT_SELECTIONS)[number];

export const RECOGNITION_PATHS = [
  "openai/gpt-transcribe",
  "openai/gpt-live-transcribe",
] as const;
export type RecognitionPath = (typeof RECOGNITION_PATHS)[number];

export const TRANSLATION_PATHS = ["openai/responses-completed-text"] as const;
export type TranslationPath = (typeof TRANSLATION_PATHS)[number];

export const TRANSLATION_TARGETS = ["en", "zh-Hans"] as const;
export type TranslationTarget = (typeof TRANSLATION_TARGETS)[number];

export const TRANSLATION_ENDPOINT_KINDS = ["official", "custom"] as const;

export const RECOGNITION_INPUT_SHAPES = ["continuousAudioFrames"] as const;
export type RecognitionInputShape = (typeof RECOGNITION_INPUT_SHAPES)[number];

export const TRANSLATION_INPUT_SHAPES = ["completedSourceSnapshots"] as const;
export type TranslationInputShape = (typeof TRANSLATION_INPUT_SHAPES)[number];

export const CAPTION_BOUNDARY_OWNERS = ["application"] as const;
export type CaptionBoundaryOwner = (typeof CAPTION_BOUNDARY_OWNERS)[number];

export const CAPTION_UNIT_BEHAVIORS = ["unitBased"] as const;
export type CaptionUnitBehavior = (typeof CAPTION_UNIT_BEHAVIORS)[number];

export const LANE_UPDATE_BEHAVIORS = [
  "completedOnly",
  "ongoingAndCompleted",
] as const;
export type LaneUpdateBehavior = (typeof LANE_UPDATE_BEHAVIORS)[number];

export const REVISION_BEHAVIORS = [
  "appendOnly",
  "revisableFullSnapshot",
] as const;
export type RevisionBehavior = (typeof REVISION_BEHAVIORS)[number];

export const RESOLVED_PUBLICATION_TIMINGS = ["completed", "liveUnit"] as const;
export const PUBLICATION_PLAN_STATES = ["compatible", "incompatible"] as const;
export const PUBLICATION_INCOMPATIBILITY_REASONS = [
  "noLanesSelected",
  "laneUnavailable",
  "modeUnsupported",
] as const;

export type RecognitionCapabilityProfile = Readonly<{
  path: RecognitionPath;
  inputShape: RecognitionInputShape;
  captionBoundaryOwner: CaptionBoundaryOwner;
  unitBehavior: CaptionUnitBehavior;
  lanes: readonly Readonly<{
    lane: CaptionLane;
    updates: LaneUpdateBehavior;
    revisions: RevisionBehavior;
  }>[];
}>;

export type TranslationCapabilityProfile = Readonly<{
  path: TranslationPath;
  inputShape: TranslationInputShape;
  lanes: readonly Readonly<{
    lane: CaptionLane;
    updates: LaneUpdateBehavior;
    revisions: RevisionBehavior;
  }>[];
}>;

export type ResolvedPublicationTiming =
  | Readonly<{ timing: "completed" }>
  | Readonly<{ timing: "liveUnit"; observationWindowMs: number }>;

export type PublicationPlan =
  | Readonly<{
      state: "compatible";
      mode: PublicationMode;
      timing: ResolvedPublicationTiming;
      selectedLanes: readonly CaptionLane[];
    }>
  | Readonly<{
      state: "incompatible";
      requestedMode: PublicationMode;
      selectedLanes: readonly CaptionLane[];
      reason:
        | Readonly<{ reason: "noLanesSelected" }>
        | Readonly<{
            reason: "laneUnavailable" | "modeUnsupported";
            lanes: readonly CaptionLane[];
          }>;
      supportedModes: readonly PublicationMode[];
    }>;

export type CaptionPipelinePlan = Readonly<{
  recognition: RecognitionCapabilityProfile;
  translation: TranslationCapabilityProfile | null;
  publication: PublicationPlan;
}>;

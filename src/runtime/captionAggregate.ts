import type { CaptionPreviewStatus } from "./presentation";

export const CAPTION_AGGREGATE_CONTRACT_VERSION = 2 as const;

export const CAPTION_LANES = ["source", "translation"] as const;
export type CaptionLane = (typeof CAPTION_LANES)[number];

export const CAPTION_STATES = ["ongoing", "completed"] as const;
export type CaptionState = (typeof CAPTION_STATES)[number];

export type SourceSnapshotRef = Readonly<{
  generation: number;
  streamId: string;
  unitId: string;
  revision: number;
}>;

export const TRANSLATION_UNIT_STATES = [
  "pending",
  "completed",
  "failed",
] as const;
export type TranslationUnitState = (typeof TRANSLATION_UNIT_STATES)[number];

export const TRANSLATION_FAILURE_REASONS = [
  "translation.provider_authentication_failed",
  "translation.provider_permission_denied",
  "translation.provider_invalid_request",
  "translation.provider_rate_limited",
  "translation.provider_usage_limit",
  "translation.provider_unavailable",
  "translation.invalid_output",
  "translation.deadline_exceeded",
  "translation.backpressure",
  "translation.source_too_large",
  "translation.stopped",
  "translation.failed",
] as const;
export type TranslationFailureReason =
  (typeof TRANSLATION_FAILURE_REASONS)[number];

export type TranslationUnitSnapshot =
  | Readonly<{
      state: "pending";
      sourceRef: SourceSnapshotRef;
    }>
  | Readonly<{
      state: "completed";
      sourceRef: SourceSnapshotRef;
    }>
  | Readonly<{
      state: "failed";
      sourceRef: SourceSnapshotRef;
      reasonCode: TranslationFailureReason;
    }>;

export type CaptionSnapshot = Readonly<{
  generation: number;
  streamId: string;
  unitId: string | null;
  lane: CaptionLane;
  revision: number;
  text: string;
  state: CaptionState;
  language: string | null;
  sourceRef: SourceSnapshotRef | null;
  unitStartedAtMs: number | null;
  timestampMs: number;
}>;

export type OpenSourceUnit = Readonly<{
  unitId: string;
  startedAtMs: number;
}>;

export type CaptionAggregateSnapshot = Readonly<{
  contractVersion: typeof CAPTION_AGGREGATE_CONTRACT_VERSION;
  snapshotRevision: number;
  activeStream: Readonly<{
    generation: number;
    streamId: string;
  }> | null;
  openSourceUnits: readonly OpenSourceUnit[];
  captions: readonly CaptionSnapshot[];
  translationUnits: readonly TranslationUnitSnapshot[];
}>;

export type CaptionDisplay = CaptionSnapshot & Readonly<{ id: string }>;

export type CaptionAggregateView = Readonly<{
  captionPreviewStatus: CaptionPreviewStatus;
  visibleCaption: CaptionSnapshot | null;
  completedCaptions: readonly CaptionSnapshot[];
}>;

const COMPLETED_CAPTION_DISPLAY_LIMIT = 5;

type CaptionAdmission = "open" | "stopped" | "awaitingStartSnapshot";

export type CaptionAggregateState = Readonly<{
  snapshot: CaptionAggregateSnapshot | null;
  admission: CaptionAdmission;
  admissionBeforeStop: Exclude<CaptionAdmission, "stopped"> | null;
  highestGenerationSeen: number;
}>;

export type CaptionAggregateStateInput =
  | Readonly<{
      type: "snapshotReceived";
      snapshot: CaptionAggregateSnapshot;
    }>
  | Readonly<{ type: "stopRequested" }>
  | Readonly<{ type: "stopFailed" }>
  | Readonly<{ type: "startSucceeded" }>;

export function createCaptionAggregateState(): CaptionAggregateState {
  return {
    snapshot: null,
    admission: "open",
    admissionBeforeStop: null,
    highestGenerationSeen: 0,
  };
}

export function reduceCaptionAggregateState(
  state: CaptionAggregateState,
  input: CaptionAggregateStateInput,
): CaptionAggregateState {
  if (input.type === "stopRequested") {
    return state.admission === "stopped"
      ? state
      : {
          ...state,
          admission: "stopped",
          admissionBeforeStop: state.admission,
        };
  }

  if (input.type === "stopFailed") {
    return state.admission === "stopped" && state.admissionBeforeStop !== null
      ? {
          ...state,
          admission: state.admissionBeforeStop,
          admissionBeforeStop: null,
        }
      : state;
  }

  if (input.type === "startSucceeded") {
    return state.admission === "open" && state.snapshot?.activeStream != null
      ? state
      : {
          ...state,
          admission: "awaitingStartSnapshot",
          admissionBeforeStop: null,
        };
  }

  if (
    state.snapshot !== null &&
    input.snapshot.snapshotRevision <= state.snapshot.snapshotRevision
  ) {
    return state;
  }

  if (state.admission === "stopped" && input.snapshot.activeStream !== null) {
    return state;
  }

  if (
    input.snapshot.activeStream !== null &&
    input.snapshot.activeStream.generation < state.highestGenerationSeen
  ) {
    return state;
  }
  if (
    state.admission === "awaitingStartSnapshot" &&
    input.snapshot.activeStream !== null &&
    input.snapshot.activeStream.generation <= state.highestGenerationSeen
  ) {
    return state;
  }

  const admission =
    state.admission === "awaitingStartSnapshot" &&
    input.snapshot.activeStream !== null
      ? "open"
      : state.admission;

  return {
    ...state,
    admission,
    admissionBeforeStop:
      admission === "stopped" ? state.admissionBeforeStop : null,
    snapshot: input.snapshot,
    highestGenerationSeen: Math.max(
      state.highestGenerationSeen,
      input.snapshot.activeStream?.generation ?? 0,
    ),
  };
}

export function selectCaptionAggregateView(
  state: CaptionAggregateState,
  showOngoing: boolean,
): CaptionAggregateView {
  const captions = state.snapshot?.captions ?? [];
  const completedCaptions = captions
    .filter(
      (caption) => caption.lane === "source" && caption.state === "completed",
    )
    .slice(0, COMPLETED_CAPTION_DISPLAY_LIMIT);
  const aggregateIsAdmitted = state.admission === "open";
  const ongoingCaption =
    aggregateIsAdmitted && showOngoing
      ? (captions.find(
          (caption) => caption.lane === "source" && caption.state === "ongoing",
        ) ?? null)
      : null;
  const hasOpenUnit =
    aggregateIsAdmitted && (state.snapshot?.openSourceUnits.length ?? 0) > 0;

  return {
    captionPreviewStatus: ongoingCaption
      ? "ongoing"
      : hasOpenUnit
        ? "listening"
        : completedCaptions.length > 0
          ? "completed"
          : "waiting",
    visibleCaption: ongoingCaption ?? completedCaptions[0] ?? null,
    completedCaptions,
  };
}

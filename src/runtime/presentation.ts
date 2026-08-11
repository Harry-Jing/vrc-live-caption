// Shared semantic presentation mappings for runtime state shown in the UI.
// Each map covers its full union type, so adding a status, mode, category, or
// service path without choosing its presentation fails the typecheck instead of
// silently rendering a default.

import { uiText, type UiStaticMessageKey } from "../i18n/uiText";
import type {
  CaptionPipelinePlan,
  PublicationMode,
  RecognitionPath,
} from "./captionPipeline";
import type {
  DiagnosticCategory,
  DiagnosticSeverity,
  RuntimeStatus,
} from "./runtimeEvents";

export type CaptionPreviewStatus =
  "waiting" | "listening" | "ongoing" | "completed";

type StatusBadgeColor = "error" | "info" | "neutral" | "success" | "warning";

// Traffic-light semantics: green while running, red on error, info for
// expected transitional states, and calm neutral for resting states.
export const runtimeStatusColor: Record<RuntimeStatus, StatusBadgeColor> = {
  idle: "neutral",
  starting: "info",
  running: "success",
  reconnecting: "warning",
  stopping: "info",
  stopped: "neutral",
  error: "error",
};

export const captionPreviewStatusColor: Record<
  CaptionPreviewStatus,
  StatusBadgeColor
> = {
  waiting: "neutral",
  listening: "info",
  ongoing: "warning",
  completed: "success",
};

export const diagnosticSeverityColor: Record<
  DiagnosticSeverity,
  StatusBadgeColor
> = {
  info: "info",
  warning: "warning",
  error: "error",
};

export const runtimeStatusMessageKey = {
  idle: "runtime.status.idle",
  starting: "runtime.status.starting",
  running: "runtime.status.running",
  reconnecting: "runtime.status.reconnecting",
  stopping: "runtime.status.stopping",
  stopped: "runtime.status.stopped",
  error: "runtime.status.error",
} satisfies Record<RuntimeStatus, UiStaticMessageKey>;

export const captionPreviewStatusMessageKey = {
  waiting: "caption.previewStatus.waiting",
  listening: "caption.previewStatus.listening",
  ongoing: "caption.previewStatus.ongoing",
  completed: "caption.previewStatus.completed",
} satisfies Record<CaptionPreviewStatus, UiStaticMessageKey>;

export const captionPreviewStatusIcon = {
  waiting: "i-lucide-clock-3",
  listening: "i-lucide-audio-lines",
  ongoing: "i-lucide-message-square-more",
  completed: "i-lucide-circle-check",
} satisfies Record<CaptionPreviewStatus, string>;

export const diagnosticSeverityMessageKey = {
  info: "diagnostics.severity.info",
  warning: "diagnostics.severity.warning",
  error: "diagnostics.severity.error",
} satisfies Record<DiagnosticSeverity, UiStaticMessageKey>;

export const diagnosticCategoryMessageKey = {
  config: "diagnostics.category.config",
  runtime: "diagnostics.category.runtime",
  audio: "diagnostics.category.audio",
  stt: "diagnostics.category.stt",
  osc: "diagnostics.category.osc",
} satisfies Record<DiagnosticCategory, UiStaticMessageKey>;

export const recognitionPathServiceMessageKey = {
  "openai/gpt-transcribe": "serviceProvider.openai",
  "openai/gpt-live-transcribe": "serviceProvider.openai",
} satisfies Record<RecognitionPath, UiStaticMessageKey>;

export const recognitionPathMessageKey = {
  "openai/gpt-transcribe": "recognition.path.gptTranscribe",
  "openai/gpt-live-transcribe": "recognition.path.gptLiveTranscribe",
} satisfies Record<RecognitionPath, UiStaticMessageKey>;

export const recognitionPathDescriptionMessageKey = {
  "openai/gpt-transcribe": "recognition.path.gptTranscribe.description",
  "openai/gpt-live-transcribe":
    "recognition.path.gptLiveTranscribe.description",
} satisfies Record<RecognitionPath, UiStaticMessageKey>;

export const publicationModeMessageKey = {
  completed: "publication.mode.completed",
  live: "publication.mode.live",
} satisfies Record<PublicationMode, UiStaticMessageKey>;

export const publicationModeDescriptionMessageKey = {
  completed: "publication.option.completed.description",
  live: "publication.option.live.description",
} satisfies Record<PublicationMode, UiStaticMessageKey>;

export type PublicationPlanView =
  | Readonly<{ state: "unavailable" }>
  | Readonly<{
      state: "compatible";
      mode: PublicationMode;
      timing: "completed";
      delayMs: null;
    }>
  | Readonly<{
      state: "compatible";
      mode: PublicationMode;
      timing: "liveUnit";
      delayMs: number;
    }>
  | Readonly<{
      state: "incompatible";
      mode: PublicationMode;
      supportedModes: readonly PublicationMode[];
    }>;

export type PublicationSettingsView =
  Readonly<{ state: "unverified" }> | PublicationPlanView;

export type PublicationCompatibleView = Extract<
  PublicationPlanView,
  Readonly<{ state: "compatible" }>
>;

export function publicationPlanView(
  captionPipelinePlan: CaptionPipelinePlan | null,
): PublicationPlanView {
  if (captionPipelinePlan === null) {
    return { state: "unavailable" };
  }

  const { publication } = captionPipelinePlan;

  if (publication.state === "incompatible") {
    return {
      state: "incompatible",
      mode: publication.requestedMode,
      supportedModes: publication.supportedModes,
    };
  }

  if (publication.timing.timing === "completed") {
    return {
      state: "compatible",
      mode: publication.mode,
      timing: "completed",
      delayMs: null,
    };
  }

  return {
    state: "compatible",
    mode: publication.mode,
    timing: "liveUnit",
    delayMs: publication.timing.observationWindowMs,
  };
}

export function publicationDisplayPlanView(
  currentGenerationCaptionPipelinePlan: CaptionPipelinePlan | null,
  desiredCaptionPipelinePlan: CaptionPipelinePlan | null,
): PublicationPlanView {
  return publicationPlanView(
    currentGenerationCaptionPipelinePlan ?? desiredCaptionPipelinePlan,
  );
}

export function publicationSettingsView(
  desiredCaptionPipelinePlan: CaptionPipelinePlan | null,
  isFormDirty: boolean,
): PublicationSettingsView {
  if (isFormDirty) {
    return { state: "unverified" };
  }

  return publicationPlanView(desiredCaptionPipelinePlan);
}

export function publicationPlanDescription(
  plan: PublicationCompatibleView,
): string {
  switch (plan.timing) {
    case "completed":
      return uiText("publication.timing.completed");
    case "liveUnit":
      return uiText("publication.timing.liveUnit", {
        delayMs: plan.delayMs,
      });
  }
}

export function publicationStartIsBlocked(
  hasActiveGeneration: boolean,
  desiredCaptionPipelinePlan: CaptionPipelinePlan | null,
): boolean {
  return (
    !hasActiveGeneration &&
    desiredCaptionPipelinePlan?.publication.state === "incompatible"
  );
}

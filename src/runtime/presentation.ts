// Shared semantic presentation mappings for runtime state shown in the UI.
// Each map covers its full union type, so adding a status, mode, category, or
// provider without choosing its presentation fails the typecheck instead of
// silently rendering a default.

import { uiText, type UiStaticMessageKey } from "../i18n/uiText";
import type {
  CaptionMode,
  DiagnosticCategory,
  DiagnosticSeverity,
  OpenAiTranscriptionModel,
  PublicationMode,
  RuntimePlan,
  RuntimeStatus,
  SttProvider,
} from "./types";

type StatusBadgeColor = "error" | "info" | "neutral" | "success" | "warning";

// Traffic-light semantics: green while running, red on error, info for
// expected transitional states, and calm neutral for resting states.
export const runtimeStatusColor: Record<RuntimeStatus, StatusBadgeColor> = {
  idle: "neutral",
  starting: "info",
  running: "success",
  stopping: "info",
  stopped: "neutral",
  error: "error",
};

export const captionModeColor: Record<CaptionMode, StatusBadgeColor> = {
  waiting: "neutral",
  listening: "info",
  partial: "warning",
  final: "success",
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
  stopping: "runtime.status.stopping",
  stopped: "runtime.status.stopped",
  error: "runtime.status.error",
} satisfies Record<RuntimeStatus, UiStaticMessageKey>;

export const captionModeMessageKey = {
  waiting: "caption.mode.waiting",
  listening: "caption.mode.listening",
  partial: "caption.mode.partial",
  final: "caption.mode.final",
} satisfies Record<CaptionMode, UiStaticMessageKey>;

export const captionModeIcon = {
  waiting: "i-lucide-clock-3",
  listening: "i-lucide-audio-lines",
  partial: "i-lucide-message-square-more",
  final: "i-lucide-circle-check",
} satisfies Record<CaptionMode, string>;

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

export const sttProviderMessageKey = {
  openai: "stt.providers.openai",
} satisfies Record<SttProvider, UiStaticMessageKey>;

export const openAiTranscriptionModelMessageKey = {
  "gpt-transcribe": "stt.models.gptTranscribe",
  "gpt-live-transcribe": "stt.models.gptLiveTranscribe",
} satisfies Record<OpenAiTranscriptionModel, UiStaticMessageKey>;

export const openAiTranscriptionModelDescriptionMessageKey = {
  "gpt-transcribe": "stt.models.gptTranscribe.description",
  "gpt-live-transcribe": "stt.models.gptLiveTranscribe.description",
} satisfies Record<OpenAiTranscriptionModel, UiStaticMessageKey>;

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
      state: "ready";
      mode: PublicationMode;
      policy: "completed";
      delayMs: null;
    }>
  | Readonly<{
      state: "ready";
      mode: PublicationMode;
      policy: "liveUnit";
      delayMs: number;
    }>
  | Readonly<{
      state: "incompatible";
      mode: PublicationMode;
      supportedModes: readonly PublicationMode[];
    }>;

export type PublicationSettingsView =
  Readonly<{ state: "unverified" }> | PublicationPlanView;

export type PublicationReadyView = Extract<
  PublicationPlanView,
  Readonly<{ state: "ready" }>
>;

export function publicationPlanView(
  runtimePlan: RuntimePlan | null,
): PublicationPlanView {
  if (runtimePlan === null) {
    return { state: "unavailable" };
  }

  const { publication } = runtimePlan;

  if (publication.state === "incompatible") {
    return {
      state: "incompatible",
      mode: publication.requestedMode,
      supportedModes: publication.supportedModes,
    };
  }

  if (publication.policy.policy === "completed") {
    return {
      state: "ready",
      mode: publication.mode,
      policy: "completed",
      delayMs: null,
    };
  }

  return {
    state: "ready",
    mode: publication.mode,
    policy: "liveUnit",
    delayMs: publication.policy.observationWindowMs,
  };
}

export function publicationDisplayPlanView(
  activeRuntimePlan: RuntimePlan | null,
  desiredRuntimePlan: RuntimePlan | null,
): PublicationPlanView {
  return publicationPlanView(activeRuntimePlan ?? desiredRuntimePlan);
}

export function publicationSettingsView(
  desiredRuntimePlan: RuntimePlan | null,
  isFormDirty: boolean,
): PublicationSettingsView {
  if (isFormDirty) {
    return { state: "unverified" };
  }

  return publicationPlanView(desiredRuntimePlan);
}

export function publicationPlanDescription(plan: PublicationReadyView): string {
  switch (plan.policy) {
    case "completed":
      return uiText("publication.policy.completed");
    case "liveUnit":
      return uiText("publication.policy.liveUnit", {
        delayMs: plan.delayMs,
      });
  }
}

export function publicationStartIsBlocked(
  hasActiveSession: boolean,
  desiredRuntimePlan: RuntimePlan | null,
): boolean {
  return (
    !hasActiveSession &&
    desiredRuntimePlan?.publication.state === "incompatible"
  );
}

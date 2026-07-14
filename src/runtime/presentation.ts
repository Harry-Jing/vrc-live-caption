// Shared semantic presentation mappings for runtime state shown in the UI.
// Each map covers its full union type, so adding a status, mode, category, or
// provider without choosing its presentation fails the typecheck instead of
// silently rendering a default.

import type { UiStaticMessageKey } from "../i18n/uiText";
import type {
  CaptionMode,
  DiagnosticCategory,
  DiagnosticSeverity,
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
  listening: "caption.mode.listening",
  partial: "caption.mode.partial",
  final: "caption.mode.final",
} satisfies Record<CaptionMode, UiStaticMessageKey>;

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
  mock: "stt.providers.mock",
} satisfies Record<SttProvider, UiStaticMessageKey>;

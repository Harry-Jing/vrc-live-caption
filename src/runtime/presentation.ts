// Shared semantic color mappings for runtime state shown in the UI. Each map
// is a Record over the full union type, so adding a status, mode, or severity
// without choosing a color fails the typecheck instead of silently rendering
// a default.

import type { CaptionMode, DiagnosticSeverity, RuntimeStatus } from "./types";

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

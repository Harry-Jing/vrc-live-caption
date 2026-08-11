import type { AudioLevelEvent } from "./audio";
import type { CaptionAggregateSnapshotV2 } from "./captionAggregate";

export const RUNTIME_STATUSES = [
  "idle",
  "starting",
  "running",
  "reconnecting",
  "stopping",
  "stopped",
  "error",
] as const;
export type RuntimeStatus = (typeof RUNTIME_STATUSES)[number];

export const DIAGNOSTIC_CATEGORIES = [
  "config",
  "runtime",
  "audio",
  "stt",
  "osc",
] as const;
export type DiagnosticCategory = (typeof DIAGNOSTIC_CATEGORIES)[number];

export const DIAGNOSTIC_SEVERITIES = ["info", "warning", "error"] as const;
export type DiagnosticSeverity = (typeof DIAGNOSTIC_SEVERITIES)[number];

export type RuntimeStatusEvent = Readonly<{
  status: RuntimeStatus;
  message?: string;
  timestampMs: number;
}>;

export type DiagnosticEvent = Readonly<{
  id: string;
  category: DiagnosticCategory;
  severity: DiagnosticSeverity;
  code: string;
  message: string;
  detail?: string;
  timestampMs: number;
}>;

export type RuntimeEvent =
  | Readonly<{ type: "status"; payload: RuntimeStatusEvent }>
  | Readonly<{ type: "audioLevel"; payload: AudioLevelEvent }>
  | Readonly<{ type: "diagnostic"; payload: DiagnosticEvent }>
  | Readonly<{
      type: "captionAggregateChanged";
      payload: CaptionAggregateSnapshotV2;
    }>;

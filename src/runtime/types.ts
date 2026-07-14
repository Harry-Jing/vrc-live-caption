export type RuntimeStatus =
  "idle" | "starting" | "running" | "stopping" | "stopped" | "error";
export const STT_PROVIDERS = ["openai", "mock"] as const;
export type SttProvider = (typeof STT_PROVIDERS)[number];
export type ProviderSecretStorage = "systemCredentialStore" | "environment";
export type DiagnosticCategory = "config" | "runtime" | "audio" | "stt" | "osc";
export type DiagnosticSeverity = "info" | "warning" | "error";
export type TranscriptKind = "partial" | "stable" | "final";
export type UtteranceEndReason = "noSpeech" | "sttFailed" | "discarded";
export type CaptionMode = "listening" | "partial" | "final";

export type RuntimeCommand =
  | "start_runtime"
  | "stop_runtime"
  | "emit_mock_transcript"
  | "send_osc_test_message";

export const RUNTIME_EVENTS = {
  status: "runtime-status",
  transcriptPartial: "transcript-partial",
  transcriptFinal: "transcript-final",
  utteranceStarted: "utterance-started",
  utteranceEnded: "utterance-ended",
  diagnostic: "diagnostic-event",
} as const;

export const APP_CONFIG_SCHEMA_VERSION = 1 as const;

export type AppConfig = {
  schemaVersion: typeof APP_CONFIG_SCHEMA_VERSION;
  audio: {
    inputDeviceId: string | null;
  };
  stt: {
    provider: SttProvider;
    language: string;
    model: string;
  };
  osc: {
    host: string;
    port: number;
    enabled: boolean;
    minIntervalMs: number;
  };
  ui: {
    showPartial: boolean;
  };
};

export type AudioInputDevice = {
  id: string;
  name: string;
  isDefault: boolean;
};

export type ProviderSecretStatus = {
  provider: SttProvider;
  configured: boolean;
  storage: ProviderSecretStorage | null;
  displaySuffix: string | null;
  error: string | null;
};

export type RuntimeStatusEvent = {
  status: RuntimeStatus;
  message?: string;
  timestampMs: number;
};

export type TranscriptEvent = {
  id: string;
  utteranceId: string;
  kind: TranscriptKind;
  text: string;
  language: string;
  provider: string;
  revision: number;
  timestampMs: number;
};

export type UtteranceStartedEvent = {
  id: string;
  utteranceId: string;
  timestampMs: number;
};

export type UtteranceEndedEvent = {
  id: string;
  utteranceId: string;
  reason: UtteranceEndReason;
  timestampMs: number;
};

export type DiagnosticEvent = {
  id: string;
  category: DiagnosticCategory;
  severity: DiagnosticSeverity;
  code: string;
  message: string;
  detail?: string;
  timestampMs: number;
};

export type RuntimeStatus =
  "idle" | "starting" | "running" | "stopping" | "stopped" | "error";
export const STT_PROVIDERS = ["openai", "mock"] as const;
export type SttProvider = (typeof STT_PROVIDERS)[number];
export type ProviderSecretStorage = "systemCredentialStore" | "environment";
export type DiagnosticCategory = "config" | "runtime" | "audio" | "stt" | "osc";
export type DiagnosticSeverity = "info" | "warning" | "error";
export type UtteranceEndReason = "noSpeech" | "sttFailed" | "discarded";
export type CaptionMode = "waiting" | "listening" | "partial" | "final";

export type CaptionLane = "source" | "translation";
export type CaptionState = "ongoing" | "completed";

export type CaptionSnapshotV1 = Readonly<{
  generation: number;
  streamId: string;
  unitId: string | null;
  lane: CaptionLane;
  revision: number;
  text: string;
  state: CaptionState;
  language: string | null;
  provider: string;
  model: string;
  unitStartedAtMs: number | null;
  timestampMs: number;
}>;

export type CaptionSessionSnapshotV1 = Readonly<{
  contractVersion: 1;
  snapshotRevision: number;
  active: Readonly<{
    generation: number;
    streamId: string;
  }> | null;
  activeUnits: readonly Readonly<{
    unitId: string;
    startedAtMs: number;
  }>[];
  captions: readonly CaptionSnapshotV1[];
}>;

export type CaptionDisplay = CaptionSnapshotV1 & Readonly<{ id: string }>;

export type RuntimeCommand =
  | "start_runtime"
  | "stop_runtime"
  | "emit_mock_transcript"
  | "send_osc_test_message";

export const RUNTIME_EVENTS = {
  status: "runtime-status",
  captionSessionChanged: "caption-session-changed",
  utteranceStarted: "utterance-started",
  utteranceEnded: "utterance-ended",
  diagnostic: "diagnostic-event",
} as const;

export const RUNTIME_CONTROL_EVENT = "runtime-control-changed" as const;

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

export type RuntimePendingChange =
  "microphone" | "recognition" | "credential" | "chatboxOutput";

export type RuntimeSessionPhase = "starting" | "running" | "stopping" | "error";

export type RuntimeSessionCredential = {
  provider: SttProvider;
  storage: ProviderSecretStorage;
  displaySuffix: string | null;
  revision: number;
};

export type RuntimeSessionChatbox =
  | {
      state: "disabled";
      host: string;
      port: number;
    }
  | {
      state: "ready";
      host: string;
      port: number;
    }
  | {
      state: "unavailable";
      host: string;
      port: number;
      reasonCode: string;
    };

export type RuntimeSession = {
  generation: number;
  phase: RuntimeSessionPhase;
  startedFromConfigRevision: number;
  selected: Pick<AppConfig, "audio" | "stt" | "osc">;
  credential: RuntimeSessionCredential | null;
  chatbox: RuntimeSessionChatbox;
  uploadsMicrophoneAudio: boolean;
};

export type RuntimeControlSnapshot = {
  contractVersion: 1;
  revision: number;
  runtime: RuntimeStatusEvent;
  desired: {
    revision: number;
    config: AppConfig;
    providerSecrets: ProviderSecretStatus[];
  };
  session: RuntimeSession | null;
  pendingChanges: RuntimePendingChange[];
};

export type RuntimeStatusEvent = {
  status: RuntimeStatus;
  message?: string;
  timestampMs: number;
};

export type UtteranceStartedEvent = {
  id: string;
  generation: number;
  streamId: string;
  utteranceId: string;
  timestampMs: number;
};

export type UtteranceEndedEvent = {
  id: string;
  generation: number;
  streamId: string;
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

export type RuntimeEvent =
  | { type: "status"; payload: RuntimeStatusEvent }
  | { type: "diagnostic"; payload: DiagnosticEvent }
  | { type: "utteranceStarted"; payload: UtteranceStartedEvent }
  | { type: "utteranceEnded"; payload: UtteranceEndedEvent }
  | {
      type: "captionSessionChanged";
      payload: CaptionSessionSnapshotV1;
    };

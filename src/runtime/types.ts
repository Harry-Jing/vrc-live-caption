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
export const STT_PROVIDERS = ["openai"] as const;
export type SttProvider = (typeof STT_PROVIDERS)[number];
export const OPENAI_TRANSCRIPTION_MODELS = [
  "gpt-transcribe",
  "gpt-live-transcribe",
] as const;
export type OpenAiTranscriptionModel =
  (typeof OPENAI_TRANSCRIPTION_MODELS)[number];
export const PROVIDER_SECRET_STORAGES = [
  "systemCredentialStore",
  "environment",
] as const;
export type ProviderSecretStorage = (typeof PROVIDER_SECRET_STORAGES)[number];
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
export type CaptionPreviewStatus =
  "waiting" | "listening" | "ongoing" | "completed";

export const CAPTION_LANES = ["source", "translation"] as const;
export type CaptionLane = (typeof CAPTION_LANES)[number];
export const CAPTION_STATES = ["ongoing", "completed"] as const;
export type CaptionState = (typeof CAPTION_STATES)[number];
export const PUBLICATION_MODES = ["completed", "live"] as const;
export type PublicationMode = (typeof PUBLICATION_MODES)[number];
export const RECOGNITION_PATHS = [
  "openAiGptTranscribe",
  "openAiGptLiveTranscribe",
] as const;
export type RecognitionPath = (typeof RECOGNITION_PATHS)[number];
export const RECOGNITION_INPUT_SHAPES = ["continuousAudioFrames"] as const;
export type RecognitionInputShape = (typeof RECOGNITION_INPUT_SHAPES)[number];
export const BOUNDARY_OWNERS = ["application"] as const;
export type BoundaryOwner = (typeof BOUNDARY_OWNERS)[number];
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
export const RESOLVED_PUBLICATION_POLICIES = ["completed", "liveUnit"] as const;
export const PUBLICATION_PLAN_STATES = ["ready", "incompatible"] as const;
export const PUBLICATION_INCOMPATIBILITY_REASONS = [
  "noLanesSelected",
  "laneUnavailable",
  "modeUnsupported",
] as const;

export type RecognitionCapabilityProfile = Readonly<{
  path: RecognitionPath;
  inputShape: RecognitionInputShape;
  boundaryOwner: BoundaryOwner;
  unitBehavior: CaptionUnitBehavior;
  lanes: readonly Readonly<{
    lane: CaptionLane;
    updates: LaneUpdateBehavior;
    revisions: RevisionBehavior;
  }>[];
}>;

export type ResolvedPublicationPolicy =
  | Readonly<{ policy: "completed" }>
  | Readonly<{ policy: "liveUnit"; observationWindowMs: number }>;

export type PublicationPlan =
  | Readonly<{
      state: "ready";
      mode: PublicationMode;
      policy: ResolvedPublicationPolicy;
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

export type RuntimePlan = Readonly<{
  recognition: RecognitionCapabilityProfile;
  publication: PublicationPlan;
}>;

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
  "start_runtime" | "stop_runtime" | "send_osc_test_message";
export type RuntimeLifecycleCommand = Extract<
  RuntimeCommand,
  "start_runtime" | "stop_runtime"
>;

export const RUNTIME_EVENTS = {
  status: "runtime-status",
  audioLevel: "audio-level",
  captionSessionChanged: "caption-session-changed",
  diagnostic: "diagnostic-event",
} as const;

export const RUNTIME_CONTROL_EVENT = "runtime-control-changed" as const;

export const APP_CONFIG_SCHEMA_VERSION = 3 as const;

export type AppConfig = {
  schemaVersion: typeof APP_CONFIG_SCHEMA_VERSION;
  audio: {
    inputDeviceId: string | null;
  };
  stt: {
    provider: SttProvider;
    languages: string[];
    model: OpenAiTranscriptionModel;
  };
  osc: {
    host: string;
    port: number;
    enabled: boolean;
  };
  publication: {
    mode: PublicationMode;
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

export type AudioLevelEvent = Readonly<{
  generation: number;
  revision: number;
  rmsDbfs: number;
  peakDbfs: number;
  clipping: boolean;
  gateOpen: boolean;
  timestampMs: number;
}>;

export type AudioProbeRequest = Readonly<{
  inputDeviceId: string | null;
  durationMs: number;
}>;

export type AudioProbeResult = Readonly<{
  sampleRate: number;
  durationMs: number;
  rmsDbfs: number;
  peakDbfs: number;
  clipping: boolean;
  gateOpen: boolean;
}>;

export type ProviderSecretStatus = {
  provider: SttProvider;
  configured: boolean;
  storage: ProviderSecretStorage | null;
  displaySuffix: string | null;
  error: string | null;
};

export const RUNTIME_PENDING_CHANGES = [
  "microphone",
  "recognition",
  "credential",
  "chatboxOutput",
  "publication",
] as const;
export type RuntimePendingChange = (typeof RUNTIME_PENDING_CHANGES)[number];

export const RUNTIME_SESSION_PHASES = [
  "starting",
  "running",
  "reconnecting",
  "stopping",
  "error",
] as const;
export type RuntimeSessionPhase = (typeof RUNTIME_SESSION_PHASES)[number];
export const RUNTIME_CHATBOX_STATES = [
  "disabled",
  "ready",
  "unavailable",
] as const;

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
  selected: Pick<AppConfig, "audio" | "stt" | "osc" | "publication">;
  runtimePlan: RuntimePlan;
  credential: RuntimeSessionCredential | null;
  chatbox: RuntimeSessionChatbox;
  uploadsMicrophoneAudio: boolean;
};

export type RuntimeControlSnapshot = {
  contractVersion: 3;
  revision: number;
  runtime: RuntimeStatusEvent;
  desired: {
    revision: number;
    config: AppConfig;
    runtimePlan: RuntimePlan;
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
  | { type: "audioLevel"; payload: AudioLevelEvent }
  | { type: "diagnostic"; payload: DiagnosticEvent }
  | {
      type: "captionSessionChanged";
      payload: CaptionSessionSnapshotV1;
    };

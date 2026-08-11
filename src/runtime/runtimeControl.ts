import type { AppConfig } from "./appConfig";
import type { AppFailure } from "./appFailure";
import type { CaptionPipelinePlan } from "./captionPipeline";
import type { RuntimeStatusEvent } from "./runtimeEvents";

export const CREDENTIAL_IDS = ["openai"] as const;
export type CredentialId = (typeof CREDENTIAL_IDS)[number];

export const CREDENTIAL_STORAGES = [
  "systemCredentialStore",
  "environment",
] as const;
export type CredentialStorage = (typeof CREDENTIAL_STORAGES)[number];

export const CREDENTIAL_STATUS_STATES = [
  "unconfigured",
  "configured",
  "unavailable",
] as const;

export type CredentialStatus =
  | Readonly<{
      state: "unconfigured";
      id: CredentialId;
    }>
  | Readonly<{
      state: "configured";
      id: CredentialId;
      storage: CredentialStorage;
      displaySuffix: string | null;
    }>
  | Readonly<{
      state: "unavailable";
      id: CredentialId;
      failure: AppFailure<string>;
    }>;

export const RUNTIME_PENDING_GENERATION_CHANGES = [
  "microphone",
  "recognition",
  "credential",
  "chatboxOutput",
  "publication",
] as const;
export type RuntimePendingGenerationChange =
  (typeof RUNTIME_PENDING_GENERATION_CHANGES)[number];

export const RUNTIME_GENERATION_PHASES = [
  "starting",
  "running",
  "reconnecting",
  "stopping",
  "error",
] as const;
export type RuntimeGenerationPhase = (typeof RUNTIME_GENERATION_PHASES)[number];

export const CHATBOX_PUBLICATION_STATES = [
  "disabled",
  "ready",
  "unavailable",
] as const;

export type RuntimeGenerationCredentialSnapshot = Readonly<{
  id: CredentialId;
  storage: CredentialStorage;
  displaySuffix: string | null;
  revision: number;
}>;

export type ChatboxPublicationSnapshot =
  | Readonly<{
      state: "disabled";
      host: string;
      port: number;
    }>
  | Readonly<{
      state: "ready";
      host: string;
      port: number;
    }>
  | Readonly<{
      state: "unavailable";
      host: string;
      port: number;
      reasonCode: string;
    }>;

export type RuntimeGenerationSelection = Omit<
  AppConfig,
  "schemaVersion" | "ui"
>;

export type RuntimeGenerationSnapshot = Readonly<{
  id: number;
  phase: RuntimeGenerationPhase;
  startedFromConfigRevision: number;
  selection: RuntimeGenerationSelection;
  captionPipelinePlan: CaptionPipelinePlan;
  credential: RuntimeGenerationCredentialSnapshot | null;
  chatboxPublication: ChatboxPublicationSnapshot;
  uploadsMicrophoneAudio: boolean;
}>;

export type RuntimeControlSnapshot = Readonly<{
  contractVersion: 4;
  revision: number;
  runtimeStatus: RuntimeStatusEvent;
  desired: Readonly<{
    revision: number;
    config: AppConfig;
    captionPipelinePlan: CaptionPipelinePlan;
    credentials: readonly CredentialStatus[];
  }>;
  generation: RuntimeGenerationSnapshot | null;
  pendingGenerationChanges: readonly RuntimePendingGenerationChange[];
}>;

export type RuntimeControlProjection = Readonly<{
  desiredConfig: AppConfig | null;
  desiredCaptionPipelinePlan: CaptionPipelinePlan | null;
  currentGenerationCaptionPipelinePlan: CaptionPipelinePlan | null;
  currentGeneration: RuntimeGenerationSnapshot | null;
  currentGenerationSelection: RuntimeGenerationSelection | null;
  pendingGenerationChanges: readonly RuntimePendingGenerationChange[];
  currentGenerationUploadsMicrophoneAudio: boolean;
  credentialStatuses: Partial<Record<CredentialId, CredentialStatus>>;
}>;

export function selectNewerRuntimeControlSnapshot(
  current: RuntimeControlSnapshot | null,
  incoming: RuntimeControlSnapshot,
): RuntimeControlSnapshot {
  if (current !== null && incoming.revision <= current.revision) {
    return current;
  }

  return incoming;
}

export function runtimeStatusNeedsControlReconciliation(
  snapshot: RuntimeControlSnapshot | null,
  observedStatus: RuntimeStatusEvent,
) {
  if (snapshot === null) {
    return true;
  }

  const controlStatus = snapshot.runtimeStatus;

  if (observedStatus.timestampMs < controlStatus.timestampMs) {
    return false;
  }

  return (
    observedStatus.timestampMs > controlStatus.timestampMs ||
    observedStatus.status !== controlStatus.status ||
    observedStatus.message !== controlStatus.message
  );
}

export function projectRuntimeControlSnapshot(
  snapshot: RuntimeControlSnapshot | null,
): RuntimeControlProjection {
  if (snapshot === null) {
    return {
      desiredConfig: null,
      desiredCaptionPipelinePlan: null,
      currentGenerationCaptionPipelinePlan: null,
      currentGeneration: null,
      currentGenerationSelection: null,
      pendingGenerationChanges: [],
      currentGenerationUploadsMicrophoneAudio: false,
      credentialStatuses: {},
    };
  }

  const credentialStatuses = Object.fromEntries(
    snapshot.desired.credentials.map((status) => [status.id, status]),
  ) as Partial<Record<CredentialId, CredentialStatus>>;

  return {
    desiredConfig: snapshot.desired.config,
    desiredCaptionPipelinePlan: snapshot.desired.captionPipelinePlan,
    currentGenerationCaptionPipelinePlan:
      snapshot.generation?.captionPipelinePlan ?? null,
    currentGeneration: snapshot.generation,
    currentGenerationSelection: snapshot.generation?.selection ?? null,
    pendingGenerationChanges: snapshot.pendingGenerationChanges,
    currentGenerationUploadsMicrophoneAudio:
      snapshot.generation?.uploadsMicrophoneAudio ?? false,
    credentialStatuses,
  };
}

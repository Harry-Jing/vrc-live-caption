import type {
  AppConfig,
  ProviderSecretStatus,
  RuntimeControlSnapshot,
  RuntimePendingChange,
  RuntimeSession,
  RuntimeStatusEvent,
  SttProvider,
} from "./types";

export type RuntimeControlProjection = Readonly<{
  config: AppConfig | null;
  currentSession: RuntimeSession | null;
  currentSetupConfig: AppConfig | null;
  pendingSessionChanges: readonly RuntimePendingChange[];
  sessionUploadsMicrophoneAudio: boolean;
  secretStatuses: Partial<Record<SttProvider, ProviderSecretStatus>>;
}>;

export function reconcileRuntimeControlSnapshot(
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

  const controlStatus = snapshot.runtime;

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
      config: null,
      currentSession: null,
      currentSetupConfig: null,
      pendingSessionChanges: [],
      sessionUploadsMicrophoneAudio: false,
      secretStatuses: {},
    };
  }

  const secretStatuses = Object.fromEntries(
    snapshot.desired.providerSecrets.map((status) => [status.provider, status]),
  ) as Partial<Record<SttProvider, ProviderSecretStatus>>;
  const currentSetupConfig = snapshot.session
    ? {
        ...snapshot.desired.config,
        ...snapshot.session.selected,
      }
    : snapshot.desired.config;

  return {
    config: snapshot.desired.config,
    currentSession: snapshot.session,
    currentSetupConfig,
    pendingSessionChanges: snapshot.pendingChanges,
    sessionUploadsMicrophoneAudio:
      snapshot.session?.uploadsMicrophoneAudio ?? false,
    secretStatuses,
  };
}

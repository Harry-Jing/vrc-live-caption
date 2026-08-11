import type {
  AudioInputDevice,
  AudioProbeRequest,
  AudioProbeResult,
} from "./audio";
import type { AppConfig } from "./appConfig";
import type { CaptionAggregateSnapshotV2 } from "./captionAggregate";
import type { CredentialId, RuntimeControlSnapshot } from "./runtimeControl";
import type { RuntimeEvent } from "./runtimeEvents";

export type Unsubscribe = () => void;

export type RuntimeEventListener = (event: RuntimeEvent) => void;
export type RuntimeControlSnapshotListener = (
  snapshot: RuntimeControlSnapshot,
) => void;

export interface AppGateway {
  subscribeRuntimeEvents(listener: RuntimeEventListener): Promise<Unsubscribe>;
  subscribeRuntimeControlSnapshots(
    listener: RuntimeControlSnapshotListener,
  ): Promise<Unsubscribe>;
  sendOscTestMessage(): Promise<void>;
  startRuntime(): Promise<RuntimeControlSnapshot>;
  stopRuntime(): Promise<RuntimeControlSnapshot>;
  getRuntimeControlSnapshot(): Promise<RuntimeControlSnapshot>;
  getCaptionAggregateSnapshot(): Promise<CaptionAggregateSnapshotV2>;
  saveAppConfig(config: AppConfig): Promise<RuntimeControlSnapshot>;
  listAudioInputDevices(): Promise<AudioInputDevice[]>;
  probeAudioInput(request: AudioProbeRequest): Promise<AudioProbeResult>;
  saveCredential(
    id: CredentialId,
    secret: string,
  ): Promise<RuntimeControlSnapshot>;
  deleteCredential(id: CredentialId): Promise<RuntimeControlSnapshot>;
}

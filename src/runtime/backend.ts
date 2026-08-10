import type {
  AppConfig,
  AudioInputDevice,
  AudioProbeRequest,
  AudioProbeResult,
  CaptionSessionSnapshotV1,
  RuntimeControlSnapshot,
  RuntimeEvent,
  SttProvider,
} from "./types";

export type Unsubscribe = () => void;

export type RuntimeEventListener = (event: RuntimeEvent) => void;
export type RuntimeControlListener = (snapshot: RuntimeControlSnapshot) => void;

export interface RuntimeBackend {
  listen(listener: RuntimeEventListener): Promise<Unsubscribe>;
  listenControl(listener: RuntimeControlListener): Promise<Unsubscribe>;
  sendOscTestMessage(): Promise<void>;
  startRuntime(): Promise<RuntimeControlSnapshot>;
  stopRuntime(): Promise<RuntimeControlSnapshot>;
  getControlSnapshot(): Promise<RuntimeControlSnapshot>;
  getCaptionSessionSnapshot(): Promise<CaptionSessionSnapshotV1>;
  saveConfig(config: AppConfig): Promise<RuntimeControlSnapshot>;
  listAudioInputDevices(): Promise<AudioInputDevice[]>;
  probeAudioInput(request: AudioProbeRequest): Promise<AudioProbeResult>;
  saveProviderSecret(
    provider: SttProvider,
    secret: string,
  ): Promise<RuntimeControlSnapshot>;
  deleteProviderSecret(provider: SttProvider): Promise<RuntimeControlSnapshot>;
}

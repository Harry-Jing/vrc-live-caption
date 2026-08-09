// Backend gateway for the runtime UI. The interface is implemented once per
// environment (Tauri IPC, browser-only preview, unsupported) and selected a
// single time at startup, so composables and components never branch on the
// execution environment themselves.

import { isTauri } from "@tauri-apps/api/core";
import { uiText } from "../i18n/uiText";
import { createPreviewBackend } from "./previewBackend";
import { createTauriBackend } from "./tauriBackend";
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

function createUnsupportedBackend(): RuntimeBackend {
  const reject = () =>
    Promise.reject(new Error(uiText("runtime.errors.desktopRequired")));

  return {
    listen: reject,
    listenControl: reject,
    sendOscTestMessage: reject,
    startRuntime: reject,
    stopRuntime: reject,
    getControlSnapshot: reject,
    getCaptionSessionSnapshot: reject,
    saveConfig: reject,
    listAudioInputDevices: reject,
    probeAudioInput: reject,
    saveProviderSecret: reject,
    deleteProviderSecret: reject,
  };
}

export function createRuntimeBackend(): RuntimeBackend {
  if (isTauri()) {
    return createTauriBackend();
  }

  if (import.meta.env.DEV) {
    return createPreviewBackend();
  }

  return createUnsupportedBackend();
}

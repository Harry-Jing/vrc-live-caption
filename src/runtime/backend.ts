// Backend gateway for the runtime UI. The interface is implemented once per
// environment (Tauri IPC, browser preview mock, unsupported) and selected a
// single time at startup, so composables and components never branch on the
// execution environment themselves.

import { isTauri } from "@tauri-apps/api/core";
import { createPreviewBackend } from "./previewBackend";
import { createTauriBackend } from "./tauriBackend";
import type {
  AppConfig,
  AudioInputDevice,
  ProviderSecretStatus,
  RuntimeCommand,
  RuntimeEvent,
  RuntimeStatusEvent,
  SttProvider,
} from "./types";

export type Unsubscribe = () => void;

export type RuntimeEventListener = (event: RuntimeEvent) => void;

export interface RuntimeBackend {
  listen(listener: RuntimeEventListener): Promise<Unsubscribe>;
  runCommand(command: RuntimeCommand): Promise<void>;
  getRuntimeStatus(): Promise<RuntimeStatusEvent>;
  getConfig(): Promise<AppConfig>;
  saveConfig(config: AppConfig): Promise<AppConfig>;
  listAudioInputDevices(): Promise<AudioInputDevice[]>;
  getProviderSecretStatus(provider: SttProvider): Promise<ProviderSecretStatus>;
  saveProviderSecret(
    provider: SttProvider,
    secret: string,
  ): Promise<ProviderSecretStatus>;
  deleteProviderSecret(provider: SttProvider): Promise<ProviderSecretStatus>;
}

function createUnsupportedBackend(): RuntimeBackend {
  const reject = () =>
    Promise.reject(new Error("This feature requires the Tauri desktop app."));

  return {
    listen: reject,
    runCommand: reject,
    getRuntimeStatus: reject,
    getConfig: reject,
    saveConfig: reject,
    listAudioInputDevices: reject,
    getProviderSecretStatus: reject,
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

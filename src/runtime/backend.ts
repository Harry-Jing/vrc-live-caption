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
  DiagnosticEvent,
  ProviderSecretStatus,
  RuntimeCommand,
  RuntimeStatusEvent,
  SttProvider,
  TranscriptEvent,
  UtteranceEndedEvent,
  UtteranceStartedEvent,
} from "./types";

export type Unsubscribe = () => void;

export type RuntimeEventHandlers = {
  onStatus: (event: RuntimeStatusEvent) => void;
  onUtteranceStarted: (event: UtteranceStartedEvent) => void;
  onTranscriptPartial: (event: TranscriptEvent) => void;
  onTranscriptFinal: (event: TranscriptEvent) => void;
  onUtteranceEnded: (event: UtteranceEndedEvent) => void;
  onDiagnostic: (event: DiagnosticEvent) => void;
};

export interface RuntimeBackend {
  listen(handlers: RuntimeEventHandlers): Promise<Unsubscribe>;
  runCommand(command: RuntimeCommand): Promise<void>;
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

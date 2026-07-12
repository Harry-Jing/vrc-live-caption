import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  RuntimeBackend,
  RuntimeEventHandlers,
  Unsubscribe,
} from "./backend";
import {
  RUNTIME_EVENTS,
  type AppConfig,
  type AudioInputDevice,
  type DiagnosticEvent,
  type ProviderSecretStatus,
  type RuntimeCommand,
  type RuntimeStatusEvent,
  type SttProvider,
  type TranscriptEvent,
  type UtteranceEndedEvent,
  type UtteranceStartedEvent,
} from "./types";

export function createTauriBackend(): RuntimeBackend {
  return {
    async listen(handlers: RuntimeEventHandlers): Promise<Unsubscribe> {
      const unlisteners: UnlistenFn[] = [];
      const unsubscribe = () => {
        for (const unlisten of unlisteners.splice(0)) {
          unlisten();
        }
      };

      try {
        unlisteners.push(
          await listen<RuntimeStatusEvent>(RUNTIME_EVENTS.status, (event) => {
            handlers.onStatus(event.payload);
          }),
        );
        unlisteners.push(
          await listen<UtteranceStartedEvent>(
            RUNTIME_EVENTS.utteranceStarted,
            (event) => {
              handlers.onUtteranceStarted(event.payload);
            },
          ),
        );
        unlisteners.push(
          await listen<TranscriptEvent>(
            RUNTIME_EVENTS.transcriptPartial,
            (event) => {
              handlers.onTranscriptPartial(event.payload);
            },
          ),
        );
        unlisteners.push(
          await listen<TranscriptEvent>(
            RUNTIME_EVENTS.transcriptFinal,
            (event) => {
              handlers.onTranscriptFinal(event.payload);
            },
          ),
        );
        unlisteners.push(
          await listen<UtteranceEndedEvent>(
            RUNTIME_EVENTS.utteranceEnded,
            (event) => {
              handlers.onUtteranceEnded(event.payload);
            },
          ),
        );
        unlisteners.push(
          await listen<DiagnosticEvent>(RUNTIME_EVENTS.diagnostic, (event) => {
            handlers.onDiagnostic(event.payload);
          }),
        );
      } catch (error) {
        unsubscribe();
        throw error;
      }

      return unsubscribe;
    },

    async runCommand(command: RuntimeCommand) {
      await invoke(command);
    },

    getRuntimeStatus() {
      return invoke<RuntimeStatusEvent>("get_runtime_status");
    },

    getConfig() {
      return invoke<AppConfig>("get_app_config");
    },

    saveConfig(config: AppConfig) {
      return invoke<AppConfig>("save_app_config", { config });
    },

    listAudioInputDevices() {
      return invoke<AudioInputDevice[]>("list_audio_input_devices");
    },

    getProviderSecretStatus(provider: SttProvider) {
      return invoke<ProviderSecretStatus>("get_provider_secret_status", {
        provider,
      });
    },

    saveProviderSecret(provider: SttProvider, secret: string) {
      return invoke<ProviderSecretStatus>("save_provider_secret", {
        provider,
        secret,
      });
    },

    deleteProviderSecret(provider: SttProvider) {
      return invoke<ProviderSecretStatus>("delete_provider_secret", {
        provider,
      });
    },
  };
}

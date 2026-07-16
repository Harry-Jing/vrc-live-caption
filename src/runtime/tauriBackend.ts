import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  RuntimeBackend,
  RuntimeEventListener,
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

export type TauriBackendBridge = Readonly<{
  listen: (
    eventName: string,
    listener: (event: Readonly<{ payload: unknown }>) => void,
  ) => Promise<UnlistenFn>;
  invoke: <Result>(
    command: string,
    args?: Record<string, unknown>,
  ) => Promise<Result>;
}>;

const defaultBridge: TauriBackendBridge = {
  listen(eventName, listener) {
    return listen<unknown>(eventName, listener);
  },
  invoke,
};

export function createTauriBackend(
  bridge: TauriBackendBridge = defaultBridge,
): RuntimeBackend {
  return {
    async listen(listener: RuntimeEventListener): Promise<Unsubscribe> {
      const unlisteners: UnlistenFn[] = [];
      const unsubscribe = () => {
        for (const unlisten of unlisteners.splice(0)) {
          unlisten();
        }
      };

      function listenFor<Payload>(
        eventName: string,
        decode: (payload: unknown) => Payload,
        accept: (payload: Payload) => void,
      ) {
        return bridge.listen(eventName, (event) => {
          accept(decode(event.payload));
        });
      }

      try {
        const registrations = [
          listenFor(
            RUNTIME_EVENTS.status,
            (payload) => payload as RuntimeStatusEvent,
            (payload) => {
              listener({ type: "status", payload });
            },
          ),
          listenFor(
            RUNTIME_EVENTS.utteranceStarted,
            (payload) => payload as UtteranceStartedEvent,
            (payload) => {
              listener({ type: "utteranceStarted", payload });
            },
          ),
          listenFor(
            RUNTIME_EVENTS.transcriptPartial,
            (payload) => payload as TranscriptEvent,
            (payload) => {
              listener({ type: "transcriptPartial", payload });
            },
          ),
          listenFor(
            RUNTIME_EVENTS.transcriptFinal,
            (payload) => payload as TranscriptEvent,
            (payload) => {
              listener({ type: "transcriptFinal", payload });
            },
          ),
          listenFor(
            RUNTIME_EVENTS.utteranceEnded,
            (payload) => payload as UtteranceEndedEvent,
            (payload) => {
              listener({ type: "utteranceEnded", payload });
            },
          ),
          listenFor(
            RUNTIME_EVENTS.diagnostic,
            (payload) => payload as DiagnosticEvent,
            (payload) => {
              listener({ type: "diagnostic", payload });
            },
          ),
        ];
        const results = await Promise.allSettled(registrations);

        for (const result of results) {
          if (result.status === "fulfilled") {
            unlisteners.push(result.value);
          }
        }

        const failedRegistration = results.find(
          (result) => result.status === "rejected",
        );

        if (failedRegistration?.status === "rejected") {
          throw failedRegistration.reason;
        }
      } catch (error) {
        unsubscribe();
        throw error;
      }

      return unsubscribe;
    },

    async runCommand(command: RuntimeCommand) {
      await bridge.invoke(command);
    },

    getRuntimeStatus() {
      return bridge.invoke<RuntimeStatusEvent>("get_runtime_status");
    },

    getConfig() {
      return bridge.invoke<AppConfig>("get_app_config");
    },

    saveConfig(config: AppConfig) {
      return bridge.invoke<AppConfig>("save_app_config", { config });
    },

    listAudioInputDevices() {
      return bridge.invoke<AudioInputDevice[]>("list_audio_input_devices");
    },

    getProviderSecretStatus(provider: SttProvider) {
      return bridge.invoke<ProviderSecretStatus>("get_provider_secret_status", {
        provider,
      });
    },

    saveProviderSecret(provider: SttProvider, secret: string) {
      return bridge.invoke<ProviderSecretStatus>("save_provider_secret", {
        provider,
        secret,
      });
    },

    deleteProviderSecret(provider: SttProvider) {
      return bridge.invoke<ProviderSecretStatus>("delete_provider_secret", {
        provider,
      });
    },
  };
}

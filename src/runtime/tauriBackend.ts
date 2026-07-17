import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { decodeCaptionSessionSnapshotV1 } from "./captionSession";
import { decodeRuntimeControlSnapshotV2 } from "./runtimeControlContract";
import type {
  RuntimeBackend,
  RuntimeEventListener,
  Unsubscribe,
} from "./backend";
import {
  RUNTIME_EVENTS,
  RUNTIME_CONTROL_EVENT,
  type AppConfig,
  type AudioInputDevice,
  type DiagnosticEvent,
  type RuntimeCommand,
  type RuntimeStatusEvent,
  type SttProvider,
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
  async function invokeControlSnapshot(
    command: string,
    args?: Record<string, unknown>,
  ) {
    const payload = await bridge.invoke<unknown>(command, args);

    return decodeRuntimeControlSnapshotV2(payload);
  }

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
            RUNTIME_EVENTS.captionSessionChanged,
            decodeCaptionSessionSnapshotV1,
            (payload) => {
              listener({ type: "captionSessionChanged", payload });
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

    async listenControl(listener) {
      const unlisten = await bridge.listen(RUNTIME_CONTROL_EVENT, (event) => {
        listener(decodeRuntimeControlSnapshotV2(event.payload));
      });

      return () => {
        unlisten();
      };
    },

    async runCommand(command: RuntimeCommand) {
      if (command === "start_runtime" || command === "stop_runtime") {
        await invokeControlSnapshot(command);
        return;
      }

      await bridge.invoke(command);
    },

    startRuntime() {
      return invokeControlSnapshot("start_runtime");
    },

    stopRuntime() {
      return invokeControlSnapshot("stop_runtime");
    },

    getControlSnapshot() {
      return invokeControlSnapshot("get_runtime_control_snapshot");
    },

    async getCaptionSessionSnapshot() {
      const payload = await bridge.invoke<unknown>(
        "get_caption_session_snapshot",
      );

      return decodeCaptionSessionSnapshotV1(payload);
    },

    saveConfig(config: AppConfig) {
      return invokeControlSnapshot("save_app_config", {
        config,
      });
    },

    listAudioInputDevices() {
      return bridge.invoke<AudioInputDevice[]>("list_audio_input_devices");
    },

    saveProviderSecret(provider: SttProvider, secret: string) {
      return invokeControlSnapshot("save_provider_secret", {
        provider,
        secret,
      });
    },

    deleteProviderSecret(provider: SttProvider) {
      return invokeControlSnapshot("delete_provider_secret", {
        provider,
      });
    },
  };
}

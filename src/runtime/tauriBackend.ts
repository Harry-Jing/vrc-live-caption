import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { decodeAudioLevelEvent, decodeAudioProbeResult } from "./audioContract";
import { decodeCaptionSessionSnapshotV1 } from "./captionSessionContract";
import { decodeRuntimeControlSnapshotV3 } from "./runtimeControlContract";
import {
  decodeDiagnosticEvent,
  decodeRuntimeStatusEvent,
} from "./runtimeEventContract";
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
  type AudioProbeRequest,
  type SttProvider,
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

export const TAURI_COMMANDS = {
  saveAppConfig: "save_app_config",
  listAudioInputDevices: "list_audio_input_devices",
  probeAudioInput: "probe_audio_input",
  startRuntime: "start_runtime",
  stopRuntime: "stop_runtime",
  getRuntimeControlSnapshot: "get_runtime_control_snapshot",
  getCaptionSessionSnapshot: "get_caption_session_snapshot",
  sendOscTestMessage: "send_osc_test_message",
  saveProviderSecret: "save_provider_secret",
  deleteProviderSecret: "delete_provider_secret",
} as const;

export function createTauriBackend(
  bridge: TauriBackendBridge = defaultBridge,
): RuntimeBackend {
  async function invokeControlSnapshot(
    command: string,
    args?: Record<string, unknown>,
  ) {
    const payload = await bridge.invoke<unknown>(command, args);

    return decodeRuntimeControlSnapshotV3(payload);
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
            decodeRuntimeStatusEvent,
            (payload) => {
              listener({ type: "status", payload });
            },
          ),
          listenFor(
            RUNTIME_EVENTS.audioLevel,
            decodeAudioLevelEvent,
            (payload) => {
              listener({ type: "audioLevel", payload });
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
            RUNTIME_EVENTS.diagnostic,
            decodeDiagnosticEvent,
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
        listener(decodeRuntimeControlSnapshotV3(event.payload));
      });

      return () => {
        unlisten();
      };
    },

    async sendOscTestMessage() {
      await bridge.invoke(TAURI_COMMANDS.sendOscTestMessage);
    },

    startRuntime() {
      return invokeControlSnapshot(TAURI_COMMANDS.startRuntime);
    },

    stopRuntime() {
      return invokeControlSnapshot(TAURI_COMMANDS.stopRuntime);
    },

    getControlSnapshot() {
      return invokeControlSnapshot(TAURI_COMMANDS.getRuntimeControlSnapshot);
    },

    async getCaptionSessionSnapshot() {
      const payload = await bridge.invoke<unknown>(
        TAURI_COMMANDS.getCaptionSessionSnapshot,
      );

      return decodeCaptionSessionSnapshotV1(payload);
    },

    saveConfig(config: AppConfig) {
      return invokeControlSnapshot(TAURI_COMMANDS.saveAppConfig, {
        config,
      });
    },

    listAudioInputDevices() {
      return bridge.invoke<AudioInputDevice[]>(
        TAURI_COMMANDS.listAudioInputDevices,
      );
    },

    async probeAudioInput(request: AudioProbeRequest) {
      const payload = await bridge.invoke<unknown>(
        TAURI_COMMANDS.probeAudioInput,
        { request },
      );

      return decodeAudioProbeResult(payload);
    },

    saveProviderSecret(provider: SttProvider, secret: string) {
      return invokeControlSnapshot(TAURI_COMMANDS.saveProviderSecret, {
        provider,
        secret,
      });
    },

    deleteProviderSecret(provider: SttProvider) {
      return invokeControlSnapshot(TAURI_COMMANDS.deleteProviderSecret, {
        provider,
      });
    },
  };
}

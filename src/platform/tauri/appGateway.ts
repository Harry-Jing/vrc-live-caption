import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  decodeAudioInputDevices,
  decodeAudioLevelEvent,
  decodeAudioProbeResult,
} from "../../runtime/wire/audioContract";
import { decodeCaptionAggregateSnapshotV2 } from "../../runtime/wire/captionAggregateContract";
import { decodeRuntimeControlSnapshotV4 } from "../../runtime/wire/runtimeControlContract";
import {
  decodeDiagnosticEvent,
  decodeRuntimeStatusEvent,
} from "../../runtime/wire/runtimeEventContract";
import {
  RUNTIME_CONTROL_EVENT,
  RUNTIME_EVENTS,
  TAURI_COMMANDS,
} from "../../runtime/wire/tauriIpc";
import type {
  AppGateway,
  RuntimeEventListener,
  Unsubscribe,
} from "../../runtime/gateway";
import type { AppConfig } from "../../runtime/appConfig";
import type { AudioProbeRequest } from "../../runtime/audio";
import type { CredentialId } from "../../runtime/runtimeControl";

export type TauriIpcBridge = Readonly<{
  listen: (
    eventName: string,
    listener: (event: Readonly<{ payload: unknown }>) => void,
  ) => Promise<UnlistenFn>;
  invoke: <Result>(
    command: string,
    args?: Record<string, unknown>,
  ) => Promise<Result>;
}>;

const defaultBridge: TauriIpcBridge = {
  listen(eventName, listener) {
    return listen<unknown>(eventName, listener);
  },
  invoke,
};

class TauriAppError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "TauriAppError";
    this.code = code;
  }
}

function normalizeTauriInvokeFailure(cause: unknown): unknown {
  if (
    typeof cause === "object" &&
    cause !== null &&
    "code" in cause &&
    typeof cause.code === "string" &&
    cause.code.length > 0 &&
    "message" in cause &&
    typeof cause.message === "string" &&
    cause.message.length > 0
  ) {
    return new TauriAppError(cause.code, cause.message);
  }

  return cause;
}

export function createTauriAppGateway(
  bridge: TauriIpcBridge = defaultBridge,
): AppGateway {
  async function invokeCommand(
    command: string,
    args?: Record<string, unknown>,
  ): Promise<unknown> {
    try {
      return await bridge.invoke<unknown>(command, args);
    } catch (cause) {
      throw normalizeTauriInvokeFailure(cause);
    }
  }

  async function invokeControlSnapshot(
    command: string,
    args?: Record<string, unknown>,
  ) {
    const payload = await invokeCommand(command, args);

    return decodeRuntimeControlSnapshotV4(payload);
  }

  return {
    async subscribeRuntimeEvents(
      listener: RuntimeEventListener,
    ): Promise<Unsubscribe> {
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
            RUNTIME_EVENTS.captionAggregateChanged,
            decodeCaptionAggregateSnapshotV2,
            (payload) => {
              listener({ type: "captionAggregateChanged", payload });
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

    async subscribeRuntimeControlSnapshots(listener) {
      const unlisten = await bridge.listen(RUNTIME_CONTROL_EVENT, (event) => {
        listener(decodeRuntimeControlSnapshotV4(event.payload));
      });

      return () => {
        unlisten();
      };
    },

    async sendOscTestMessage() {
      await invokeCommand(TAURI_COMMANDS.sendOscTestMessage);
    },

    startRuntime() {
      return invokeControlSnapshot(TAURI_COMMANDS.startRuntime);
    },

    stopRuntime() {
      return invokeControlSnapshot(TAURI_COMMANDS.stopRuntime);
    },

    getRuntimeControlSnapshot() {
      return invokeControlSnapshot(TAURI_COMMANDS.getRuntimeControlSnapshot);
    },

    async getCaptionAggregateSnapshot() {
      const payload = await invokeCommand(
        TAURI_COMMANDS.getCaptionAggregateSnapshot,
      );

      return decodeCaptionAggregateSnapshotV2(payload);
    },

    saveAppConfig(config: AppConfig) {
      return invokeControlSnapshot(TAURI_COMMANDS.saveAppConfig, {
        config,
      });
    },

    async listAudioInputDevices() {
      const payload = await invokeCommand(TAURI_COMMANDS.listAudioInputDevices);

      return decodeAudioInputDevices(payload);
    },

    async probeAudioInput(request: AudioProbeRequest) {
      const payload = await invokeCommand(TAURI_COMMANDS.probeAudioInput, {
        request,
      });

      return decodeAudioProbeResult(payload);
    },

    saveCredential(id: CredentialId, secret: string) {
      return invokeControlSnapshot(TAURI_COMMANDS.saveCredential, {
        id,
        secret,
      });
    },

    deleteCredential(id: CredentialId) {
      return invokeControlSnapshot(TAURI_COMMANDS.deleteCredential, {
        id,
      });
    },
  };
}

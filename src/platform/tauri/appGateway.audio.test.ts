import { expect, test } from "vitest";
import type { AudioLevelEvent } from "../../runtime/audio";
import { RUNTIME_EVENTS, TAURI_COMMANDS } from "../../runtime/wire/tauriIpc";
import { createTauriAppGateway, type TauriIpcBridge } from "./appGateway";

test("Tauri AppGateway decodes realtime audio levels before delivery", async () => {
  let deliverAudioLevel:
    ((event: Readonly<{ payload: unknown }>) => void) | undefined;
  const bridge: TauriIpcBridge = {
    listen(eventName, listener) {
      if (eventName === RUNTIME_EVENTS.audioLevel) {
        deliverAudioLevel = listener;
      }

      return Promise.resolve(() => undefined);
    },
    invoke<Result>() {
      return Promise.resolve(undefined as Result);
    },
  };
  const gateway = createTauriAppGateway(bridge);
  const received: AudioLevelEvent[] = [];
  const unsubscribe = await gateway.subscribeRuntimeEvents((event) => {
    if (event.type === "audioLevel") {
      received.push(event.payload);
    }
  });
  const payload = {
    generation: 4,
    revision: 9,
    rmsDbfs: -32,
    peakDbfs: -6,
    clipping: false,
    gateOpen: true,
    timestampMs: 5_000,
  };

  deliverAudioLevel?.({ payload });
  unsubscribe();

  expect(received).toEqual([payload]);
});

test("Tauri AppGateway invokes and decodes an offline audio probe", async () => {
  const invocations: {
    command: string;
    args: Record<string, unknown> | undefined;
  }[] = [];
  const payload = {
    sampleRate: 48_000,
    durationMs: 2_500,
    rmsDbfs: -29,
    peakDbfs: -5,
    clipping: false,
    gateOpen: true,
  };
  const bridge: TauriIpcBridge = {
    listen() {
      return Promise.resolve(() => undefined);
    },
    invoke<Result>(command: string, args?: Record<string, unknown>) {
      invocations.push({ command, args });
      return Promise.resolve(payload as Result);
    },
  };
  const gateway = createTauriAppGateway(bridge);
  const request = { inputDeviceId: null, durationMs: 2_500 } as const;

  await expect(gateway.probeAudioInput(request)).resolves.toEqual(payload);
  expect(invocations).toEqual([
    { command: TAURI_COMMANDS.probeAudioInput, args: { request } },
  ]);
});

test("Tauri AppGateway rejects a malformed realtime audio level", async () => {
  let deliverAudioLevel:
    ((event: Readonly<{ payload: unknown }>) => void) | undefined;
  const bridge: TauriIpcBridge = {
    listen(eventName, listener) {
      if (eventName === RUNTIME_EVENTS.audioLevel) {
        deliverAudioLevel = listener;
      }
      return Promise.resolve(() => undefined);
    },
    invoke<Result>() {
      return Promise.resolve(undefined as Result);
    },
  };
  const gateway = createTauriAppGateway(bridge);
  const received: AudioLevelEvent[] = [];
  const unsubscribe = await gateway.subscribeRuntimeEvents((event) => {
    if (event.type === "audioLevel") {
      received.push(event.payload);
    }
  });

  expect(() =>
    deliverAudioLevel?.({
      payload: {
        generation: 1,
        revision: 1,
        rmsDbfs: "quiet",
        peakDbfs: -6,
        clipping: false,
        gateOpen: false,
        timestampMs: 1,
      },
    }),
  ).toThrow("$.rmsDbfs");
  expect(received).toEqual([]);
  unsubscribe();
});

test("Tauri AppGateway rejects malformed audio input devices", async () => {
  const bridge: TauriIpcBridge = {
    listen() {
      return Promise.resolve(() => undefined);
    },
    invoke<Result>(command: string) {
      if (command !== TAURI_COMMANDS.listAudioInputDevices) {
        return Promise.reject(new Error(`Unexpected command: ${command}`));
      }

      return Promise.resolve([
        { id: "default", name: "Default microphone", isDefault: "yes" },
      ] as Result);
    },
  };
  const gateway = createTauriAppGateway(bridge);

  await expect(gateway.listAudioInputDevices()).rejects.toThrow(
    "$[0].isDefault",
  );
});

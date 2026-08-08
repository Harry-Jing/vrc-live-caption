import { expect, test } from "vitest";
import { createTauriBackend, type TauriBackendBridge } from "./tauriBackend";
import { RUNTIME_EVENTS, type AudioLevelEvent } from "./types";

test("TauriBackend decodes realtime audio levels before delivery", async () => {
  let deliverAudioLevel:
    ((event: Readonly<{ payload: unknown }>) => void) | undefined;
  const bridge: TauriBackendBridge = {
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
  const backend = createTauriBackend(bridge);
  const received: AudioLevelEvent[] = [];
  const unsubscribe = await backend.listen((event) => {
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

test("TauriBackend invokes and decodes an offline audio probe", async () => {
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
  const bridge: TauriBackendBridge = {
    listen() {
      return Promise.resolve(() => undefined);
    },
    invoke<Result>(command: string, args?: Record<string, unknown>) {
      invocations.push({ command, args });
      return Promise.resolve(payload as Result);
    },
  };
  const backend = createTauriBackend(bridge);
  const request = { inputDeviceId: null, durationMs: 2_500 } as const;

  await expect(backend.probeAudioInput(request)).resolves.toEqual(payload);
  expect(invocations).toEqual([
    { command: "probe_audio_input", args: { request } },
  ]);
});

test("TauriBackend rejects a malformed realtime audio level", async () => {
  let deliverAudioLevel:
    ((event: Readonly<{ payload: unknown }>) => void) | undefined;
  const bridge: TauriBackendBridge = {
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
  const backend = createTauriBackend(bridge);
  const received: AudioLevelEvent[] = [];
  const unsubscribe = await backend.listen((event) => {
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

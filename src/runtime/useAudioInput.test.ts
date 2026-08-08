import { expect, test, vi } from "vitest";
import type { RuntimeBackend } from "./backend";
import type { AudioLevelEvent } from "./types";
import { useAudioInput } from "./useAudioInput";

function level(
  generation: number,
  revision: number,
  rmsDbfs: number,
): AudioLevelEvent {
  return {
    generation,
    revision,
    rmsDbfs,
    peakDbfs: -6,
    clipping: false,
    gateOpen: true,
    timestampMs: generation * 1_000 + revision,
  };
}

test("keeps the newest realtime audio level by generation and revision", () => {
  const backend: Pick<RuntimeBackend, "probeAudioInput"> = {
    probeAudioInput: vi.fn(),
  };
  const audio = useAudioInput(backend);

  audio.acceptAudioLevel(level(3, 2, -24));
  audio.acceptAudioLevel(level(3, 1, -40));
  audio.acceptAudioLevel(level(2, 99, -50));

  expect(audio.latestAudioLevel.value).toEqual(level(3, 2, -24));

  audio.acceptAudioLevel(level(4, 1, -18));
  expect(audio.latestAudioLevel.value).toEqual(level(4, 1, -18));
});

test("tracks an offline microphone probe from pending to result", async () => {
  let resolveProbe!: (
    result: Awaited<ReturnType<RuntimeBackend["probeAudioInput"]>>,
  ) => void;
  const pendingProbe = new Promise<
    Awaited<ReturnType<RuntimeBackend["probeAudioInput"]>>
  >((resolve) => {
    resolveProbe = resolve;
  });
  const backend: Pick<RuntimeBackend, "probeAudioInput"> = {
    probeAudioInput: vi.fn(() => pendingProbe),
  };
  const audio = useAudioInput(backend);
  const request = { inputDeviceId: "usb-headset", durationMs: 3_000 };
  const expected = {
    sampleRate: 48_000,
    durationMs: 3_000,
    rmsDbfs: -26,
    peakDbfs: -4,
    clipping: false,
    gateOpen: true,
  };

  const probe = audio.probeAudioInput(request);
  expect(audio.isAudioProbeRunning.value).toBe(true);
  expect(audio.audioProbeResult.value).toBeNull();

  resolveProbe(expected);
  await expect(probe).resolves.toEqual(expected);
  expect(audio.isAudioProbeRunning.value).toBe(false);
  expect(audio.audioProbeResult.value).toEqual(expected);
  expect(audio.audioProbeError.value).toBe("");
});

test("keeps an offline microphone probe failure in its own action state", async () => {
  const backend: Pick<RuntimeBackend, "probeAudioInput"> = {
    probeAudioInput: vi.fn(() => Promise.reject(new Error("Microphone busy"))),
  };
  const audio = useAudioInput(backend);

  await expect(
    audio.probeAudioInput({ inputDeviceId: null, durationMs: 2_000 }),
  ).resolves.toBeNull();

  expect(audio.isAudioProbeRunning.value).toBe(false);
  expect(audio.audioProbeResult.value).toBeNull();
  expect(audio.audioProbeError.value).toBe("Microphone busy");
});

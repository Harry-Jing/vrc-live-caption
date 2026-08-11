import { expect, test, vi } from "vitest";
import type { AudioLevelEvent } from "../audio";
import type { AppGateway } from "../gateway";
import { createAudioInputState } from "./audioInput";

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
  const gateway: Pick<AppGateway, "probeAudioInput"> = {
    probeAudioInput: vi.fn(),
  };
  const audio = createAudioInputState(gateway);

  audio.acceptAudioLevel(level(3, 2, -24));
  audio.acceptAudioLevel(level(3, 1, -40));
  audio.acceptAudioLevel(level(2, 99, -50));

  expect(audio.latestAudioLevel.value).toEqual(level(3, 2, -24));

  audio.acceptAudioLevel(level(4, 1, -18));
  expect(audio.latestAudioLevel.value).toEqual(level(4, 1, -18));
});

test("tracks an offline microphone probe from pending to result", async () => {
  let resolveProbe!: (
    result: Awaited<ReturnType<AppGateway["probeAudioInput"]>>,
  ) => void;
  const pendingProbe = new Promise<
    Awaited<ReturnType<AppGateway["probeAudioInput"]>>
  >((resolve) => {
    resolveProbe = resolve;
  });
  const gateway: Pick<AppGateway, "probeAudioInput"> = {
    probeAudioInput: vi.fn(() => pendingProbe),
  };
  const audio = createAudioInputState(gateway);
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
  expect(audio.audioProbeFailure.value).toBeNull();
});

test("keeps an offline microphone probe failure in its own action state", async () => {
  const gateway: Pick<AppGateway, "probeAudioInput"> = {
    probeAudioInput: vi.fn(() =>
      Promise.reject(
        Object.assign(new Error("Microphone busy"), { code: "audio.failed" }),
      ),
    ),
  };
  const audio = createAudioInputState(gateway);

  await expect(
    audio.probeAudioInput({ inputDeviceId: null, durationMs: 2_000 }),
  ).resolves.toBeNull();

  expect(audio.isAudioProbeRunning.value).toBe(false);
  expect(audio.audioProbeResult.value).toBeNull();
  expect(audio.audioProbeFailure.value).toEqual({
    code: "audio.failed",
    message: "Microphone busy",
  });
});

test("uses localized fallback copy for a non-error probe rejection", async () => {
  const gateway: Pick<AppGateway, "probeAudioInput"> = {
    probeAudioInput: vi.fn(() => {
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- Exercise the non-Error rejection fallback.
      return Promise.reject();
    }),
  };
  const audio = createAudioInputState(gateway);

  await expect(
    audio.probeAudioInput({ inputDeviceId: null, durationMs: 2_000 }),
  ).resolves.toBeNull();

  expect(audio.audioProbeFailure.value).toEqual({
    code: null,
    message: "Microphone probe failed.",
  });
});

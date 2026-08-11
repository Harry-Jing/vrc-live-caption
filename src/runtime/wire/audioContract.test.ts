import { expect, test } from "vitest";
import {
  AudioContractError,
  decodeAudioInputDevices,
  decodeAudioLevelEvent,
  decodeAudioProbeResult,
} from "./audioContract";

test("decodes an exact audio input device list", () => {
  const payload = [
    { id: "default", name: "Default microphone", isDefault: true },
    { id: "usb-headset", name: "USB headset", isDefault: false },
  ];

  expect(decodeAudioInputDevices(payload)).toEqual(payload);
});

test.each([
  ["a non-array device list", {}, "$"],
  [
    "a malformed device",
    [{ id: "usb-headset", name: "USB headset", isDefault: "yes" }],
    "$[0].isDefault",
  ],
  [
    "an unknown device field",
    [
      {
        id: "usb-headset",
        name: "USB headset",
        isDefault: false,
        hostApi: "WASAPI",
      },
    ],
    "$[0].hostApi",
  ],
] as const)("rejects %s", (_name, payload, path) => {
  expect(() => decodeAudioInputDevices(payload)).toThrow(path);
});

test("decodes a complete realtime audio level event", () => {
  const payload = {
    generation: 7,
    revision: 12,
    rmsDbfs: -31.5,
    peakDbfs: -8.25,
    clipping: false,
    gateOpen: true,
    timestampMs: 1_728_000_000_123,
  };

  expect(decodeAudioLevelEvent(payload)).toEqual(payload);
});

test("decodes a complete offline audio probe result", () => {
  const payload = {
    sampleRate: 48_000,
    durationMs: 3_000,
    rmsDbfs: -27.75,
    peakDbfs: -4.5,
    clipping: false,
    gateOpen: true,
  };

  expect(decodeAudioProbeResult(payload)).toEqual(payload);
});

test.each([
  ["unknown event field", { generation: 1, unexpected: true }, "$.unexpected"],
  [
    "non-finite realtime RMS",
    {
      generation: 1,
      revision: 1,
      rmsDbfs: Number.NaN,
      peakDbfs: -6,
      clipping: false,
      gateOpen: false,
      timestampMs: 1,
    },
    "$.rmsDbfs",
  ],
] as const)("rejects %s", (_name, payload, path) => {
  expect(() => decodeAudioLevelEvent(payload)).toThrow(path);
});

test("rejects an invalid offline probe duration", () => {
  expect(() =>
    decodeAudioProbeResult({
      sampleRate: 48_000,
      durationMs: 0,
      rmsDbfs: -30,
      peakDbfs: -6,
      clipping: false,
      gateOpen: false,
    }),
  ).toThrow("$.durationMs");
});

test("preserves the audio contract error type", () => {
  expect(() => decodeAudioLevelEvent(null)).toThrow(AudioContractError);
});

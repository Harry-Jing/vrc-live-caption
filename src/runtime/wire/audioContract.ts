import { createDecoders } from "./contractDecoding";
import type {
  AudioInputDevice,
  AudioLevelEvent,
  AudioProbeResult,
} from "../audio";

export class AudioContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`Invalid audio payload at ${path}: ${expectation}.`);
    this.name = "AudioContractError";
  }
}

const { exactRecord, array, string, safeInteger, finiteNumber, boolean } =
  createDecoders(AudioContractError);

export function decodeAudioInputDevices(value: unknown): AudioInputDevice[] {
  return array(value, "$").map((device, index) => {
    const path = `$[${String(index)}]`;
    const input = exactRecord(device, path, ["id", "name", "isDefault"]);

    return {
      id: string(input["id"], `${path}.id`),
      name: string(input["name"], `${path}.name`),
      isDefault: boolean(input["isDefault"], `${path}.isDefault`),
    };
  });
}

export function decodeAudioLevelEvent(value: unknown): AudioLevelEvent {
  const input = exactRecord(value, "$", [
    "generation",
    "revision",
    "rmsDbfs",
    "peakDbfs",
    "clipping",
    "gateOpen",
    "timestampMs",
  ]);

  return {
    generation: safeInteger(input["generation"], "$.generation", 1),
    revision: safeInteger(input["revision"], "$.revision", 1),
    rmsDbfs: finiteNumber(input["rmsDbfs"], "$.rmsDbfs"),
    peakDbfs: finiteNumber(input["peakDbfs"], "$.peakDbfs"),
    clipping: boolean(input["clipping"], "$.clipping"),
    gateOpen: boolean(input["gateOpen"], "$.gateOpen"),
    timestampMs: safeInteger(input["timestampMs"], "$.timestampMs", 0),
  };
}

export function decodeAudioProbeResult(value: unknown): AudioProbeResult {
  const input = exactRecord(value, "$", [
    "sampleRate",
    "durationMs",
    "rmsDbfs",
    "peakDbfs",
    "clipping",
    "gateOpen",
  ]);

  return {
    sampleRate: safeInteger(input["sampleRate"], "$.sampleRate", 1),
    durationMs: safeInteger(input["durationMs"], "$.durationMs", 1),
    rmsDbfs: finiteNumber(input["rmsDbfs"], "$.rmsDbfs"),
    peakDbfs: finiteNumber(input["peakDbfs"], "$.peakDbfs"),
    clipping: boolean(input["clipping"], "$.clipping"),
    gateOpen: boolean(input["gateOpen"], "$.gateOpen"),
  };
}

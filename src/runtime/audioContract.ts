import type { AudioLevelEvent, AudioProbeResult } from "./types";

export class AudioContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`Invalid audio payload at ${path}: ${expectation}.`);
    this.name = "AudioContractError";
  }
}

function exactRecord(
  value: unknown,
  path: string,
  allowedFields: readonly string[],
) {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new AudioContractError(path, "expected an object");
  }

  const record = value as Record<string, unknown>;
  const allowed = new Set(allowedFields);
  const unknownField = Object.keys(record).find((field) => !allowed.has(field));
  if (unknownField !== undefined) {
    throw new AudioContractError(`${path}.${unknownField}`, "unknown field");
  }

  return record;
}

function safeInteger(value: unknown, path: string, minimum: number) {
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < minimum
  ) {
    throw new AudioContractError(
      path,
      `expected a safe integer greater than or equal to ${String(minimum)}`,
    );
  }

  return value;
}

function finiteNumber(value: unknown, path: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new AudioContractError(path, "expected a finite number");
  }

  return value;
}

function boolean(value: unknown, path: string) {
  if (typeof value !== "boolean") {
    throw new AudioContractError(path, "expected a boolean");
  }

  return value;
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

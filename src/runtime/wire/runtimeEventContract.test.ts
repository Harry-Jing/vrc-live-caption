import { describe, expect, test } from "vitest";
import {
  RuntimeEventContractError,
  decodeDiagnosticEvent,
  decodeRuntimeStatusEvent,
} from "./runtimeEventContract";

describe("runtime status events", () => {
  test("decodes payloads with and without an optional message", () => {
    expect(
      decodeRuntimeStatusEvent({
        status: "running",
        message: "Runtime is running",
        timestampMs: 100,
      }),
    ).toEqual({
      status: "running",
      message: "Runtime is running",
      timestampMs: 100,
    });
    expect(
      decodeRuntimeStatusEvent({ status: "stopped", timestampMs: 101 }),
    ).toEqual({ status: "stopped", timestampMs: 101 });
  });

  test.each([
    [{ status: "paused", timestampMs: 1 }, "$.status"],
    [{ status: "running", timestampMs: -1 }, "$.timestampMs"],
    [{ status: "running", timestampMs: 1, extra: true }, "$.extra"],
  ] as const)("rejects malformed payloads", (payload, path) => {
    expect(() => decodeRuntimeStatusEvent(payload)).toThrow(path);
  });
});

describe("diagnostic events", () => {
  const diagnostic = {
    id: "diagnostic-1",
    category: "osc",
    severity: "warning",
    code: "osc.send_failed",
    message: "Chatbox send failed",
    detail: "Would block",
    timestampMs: 200,
  } as const;

  test("decodes payloads with and without optional detail", () => {
    expect(decodeDiagnosticEvent(diagnostic)).toEqual(diagnostic);
    const { detail, ...withoutDetail } = diagnostic;
    expect(detail).toBe("Would block");
    expect(decodeDiagnosticEvent(withoutDetail)).toEqual(withoutDetail);
  });

  test.each([
    [{ ...diagnostic, category: "network" }, "$.category"],
    [{ ...diagnostic, severity: "fatal" }, "$.severity"],
    [{ ...diagnostic, timestampMs: Number.NaN }, "$.timestampMs"],
    [{ ...diagnostic, extra: true }, "$.extra"],
    [{ ...diagnostic, code: "stt.send_failed" }, "$.code"],
  ] as const)("rejects malformed payloads", (payload, path) => {
    expect(() => decodeDiagnosticEvent(payload)).toThrow(path);
  });

  test("uses its concrete contract error type", () => {
    expect(() => decodeDiagnosticEvent(null)).toThrow(
      RuntimeEventContractError,
    );
  });
});

import { createDecoders } from "./contractDecoding";
import {
  DIAGNOSTIC_CATEGORIES,
  DIAGNOSTIC_SEVERITIES,
  RUNTIME_STATUSES,
  type DiagnosticCategory,
  type DiagnosticEvent,
  type DiagnosticSeverity,
  type RuntimeStatus,
  type RuntimeStatusEvent,
} from "../types";

export class RuntimeEventContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`Invalid runtime event payload at ${path}: ${expectation}.`);
    this.name = "RuntimeEventContractError";
  }
}

const { exactRecord, string, safeInteger, literal } = createDecoders(
  RuntimeEventContractError,
);

export function decodeRuntimeStatusEvent(value: unknown): RuntimeStatusEvent {
  const input = exactRecord(value, "$", ["status", "message", "timestampMs"]);
  const status = literal<RuntimeStatus>(
    input["status"],
    "$.status",
    RUNTIME_STATUSES,
  );
  const timestampMs = safeInteger(input["timestampMs"], "$.timestampMs", 0);

  if (!("message" in input)) {
    return { status, timestampMs };
  }

  return {
    status,
    message: string(input["message"], "$.message"),
    timestampMs,
  };
}

export function decodeDiagnosticEvent(value: unknown): DiagnosticEvent {
  const input = exactRecord(value, "$", [
    "id",
    "category",
    "severity",
    "code",
    "message",
    "detail",
    "timestampMs",
  ]);
  const category = literal<DiagnosticCategory>(
    input["category"],
    "$.category",
    DIAGNOSTIC_CATEGORIES,
  );
  const severity = literal<DiagnosticSeverity>(
    input["severity"],
    "$.severity",
    DIAGNOSTIC_SEVERITIES,
  );
  const code = string(input["code"], "$.code");

  if (!code.startsWith(`${category}.`)) {
    throw new RuntimeEventContractError(
      "$.code",
      `expected a code prefixed with ${category}.`,
    );
  }

  const base = {
    id: string(input["id"], "$.id"),
    category,
    severity,
    code,
    message: string(input["message"], "$.message"),
    timestampMs: safeInteger(input["timestampMs"], "$.timestampMs", 0),
  };

  return "detail" in input
    ? { ...base, detail: string(input["detail"], "$.detail") }
    : base;
}

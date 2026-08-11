import { beforeEach, describe, expect, test, vi } from "vitest";
import type {
  DiagnosticEvent,
  RuntimeStatusEvent,
} from "../../runtime/runtimeEvents";
import {
  copyDiagnosticReport,
  serializeDiagnosticReport,
} from "./diagnosticReport";

const hostMocks = vi.hoisted(() => ({
  resolveDiagnosticReportHost: vi.fn(),
}));

vi.mock("../../platform/diagnosticReportHost", () => ({
  resolveDiagnosticReportHost: hostMocks.resolveDiagnosticReportHost,
}));

const runtimeStatus: RuntimeStatusEvent = {
  status: "reconnecting",
  message: "Reconnecting after a temporary network failure.",
  timestampMs: 1_725_000_000_000,
};

const diagnostics: readonly DiagnosticEvent[] = [
  {
    id: "diagnostic-2",
    category: "stt",
    severity: "warning",
    code: "stt.connection_lost",
    message: "Speech recognition connection was interrupted",
    detail: "The App will retry automatically.",
    timestampMs: 1_725_000_000_100,
  },
];

beforeEach(() => {
  hostMocks.resolveDiagnosticReportHost.mockReset();
});

describe("diagnostic report", () => {
  test("projects runtime state through a strict diagnostic metadata allowlist", () => {
    const sensitiveValues = [
      "spoken caption text",
      "sk-secret-value",
      "/Users/example/private-config.json",
      "microphone-device-id-123",
      "https://private-relay.example/v1",
    ] as const;
    const runtimeStatusWithSensitiveFields = {
      ...runtimeStatus,
      message: sensitiveValues[0],
      internal: {
        configPath: sensitiveValues[2],
        device: { id: sensitiveValues[3] },
      },
    } as RuntimeStatusEvent;
    const diagnosticsWithSensitiveFields = [
      {
        ...diagnostics[0],
        message: sensitiveValues[1],
        detail: sensitiveValues[4],
        internal: {
          caption: sensitiveValues[0],
          nested: { configPath: sensitiveValues[2] },
        },
      } as DiagnosticEvent,
    ];
    const serialized = serializeDiagnosticReport({
      appVersion: "0.1.0",
      diagnostics: diagnosticsWithSensitiveFields,
      generatedAtMs: 1_725_000_001_000,
      platform: "windows",
      runtimeStatus: runtimeStatusWithSensitiveFields,
    });
    const report = JSON.parse(serialized) as Record<string, unknown>;

    expect(report).toEqual({
      reportVersion: 1,
      generatedAt: "2024-08-30T06:40:01.000Z",
      appVersion: "0.1.0",
      platform: "windows",
      runtimeStatus: {
        status: "reconnecting",
        timestampMs: 1_725_000_000_000,
      },
      diagnostics: [
        {
          category: "stt",
          severity: "warning",
          code: "stt.connection_lost",
          timestampMs: 1_725_000_000_100,
        },
      ],
    });
    for (const sensitiveValue of sensitiveValues) {
      expect(serialized).not.toContain(sensitiveValue);
    }
    expect(serialized).not.toContain("diagnostic-2");
    expect(report).not.toHaveProperty("captions");
    expect(report).not.toHaveProperty("config");
    expect(report).not.toHaveProperty("serviceCredentials");
  });

  test("writes through the resolved host with host-owned app metadata", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    hostMocks.resolveDiagnosticReportHost.mockResolvedValue({
      appVersion: "0.1.0",
      userAgent: "Windows / WebView2",
      writeText,
    });

    await copyDiagnosticReport(
      { diagnostics, runtimeStatus },
      1_725_000_001_000,
    );

    expect(hostMocks.resolveDiagnosticReportHost).toHaveBeenCalledOnce();
    expect(writeText).toHaveBeenCalledOnce();
    expect(writeText.mock.calls[0]?.[0]).toContain('"appVersion": "0.1.0"');
    expect(writeText.mock.calls[0]?.[0]).toContain('"platform": "windows"');
  });

  test("projects unknown host user agents without leaking platform details", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    hostMocks.resolveDiagnosticReportHost.mockResolvedValue({
      appVersion: "preview",
      userAgent: "Browser preview",
      writeText,
    });

    await copyDiagnosticReport(
      { diagnostics, runtimeStatus },
      1_725_000_001_000,
    );

    expect(writeText).toHaveBeenCalledOnce();
    expect(writeText.mock.calls[0]?.[0]).toContain('"appVersion": "preview"');
    expect(writeText.mock.calls[0]?.[0]).toContain('"platform": "unknown"');
  });
});

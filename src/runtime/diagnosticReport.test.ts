import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { DiagnosticEvent, RuntimeStatusEvent } from "./types";
import {
  copyDiagnosticReport,
  serializeDiagnosticReport,
} from "./diagnosticReport";

const platformMocks = vi.hoisted(() => ({
  getVersion: vi.fn(),
  isTauri: vi.fn(),
  writeText: vi.fn(),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: platformMocks.getVersion,
}));

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: platformMocks.isTauri,
}));

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: platformMocks.writeText,
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
  platformMocks.getVersion.mockReset();
  platformMocks.isTauri.mockReset();
  platformMocks.writeText.mockReset();
});

afterEach(() => {
  vi.unstubAllGlobals();
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
    expect(report).not.toHaveProperty("providerSecrets");
  });

  test("uses the native write-only clipboard path in Tauri", async () => {
    platformMocks.isTauri.mockReturnValue(true);
    platformMocks.getVersion.mockResolvedValue("0.1.0");
    platformMocks.writeText.mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { userAgent: "Windows / WebView2" });

    await copyDiagnosticReport(
      { diagnostics, runtimeStatus },
      1_725_000_001_000,
    );

    expect(platformMocks.getVersion).toHaveBeenCalledOnce();
    expect(platformMocks.writeText).toHaveBeenCalledOnce();
    expect(platformMocks.writeText.mock.calls[0]?.[0]).toContain(
      '"appVersion": "0.1.0"',
    );
    expect(platformMocks.writeText.mock.calls[0]?.[0]).toContain(
      '"platform": "windows"',
    );
  });

  test("uses the browser clipboard in preview mode", async () => {
    const browserWriteText = vi.fn().mockResolvedValue(undefined);
    platformMocks.isTauri.mockReturnValue(false);
    vi.stubGlobal("navigator", {
      clipboard: { writeText: browserWriteText },
      userAgent: "Browser preview",
    });

    await copyDiagnosticReport(
      { diagnostics, runtimeStatus },
      1_725_000_001_000,
    );

    expect(platformMocks.getVersion).not.toHaveBeenCalled();
    expect(platformMocks.writeText).not.toHaveBeenCalled();
    expect(browserWriteText).toHaveBeenCalledOnce();
    expect(browserWriteText.mock.calls[0]?.[0]).toContain(
      '"appVersion": "preview"',
    );
    expect(browserWriteText.mock.calls[0]?.[0]).toContain(
      '"platform": "unknown"',
    );
  });
});

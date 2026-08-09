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
  test("serializes only redacted runtime status and diagnostic events", () => {
    const report = JSON.parse(
      serializeDiagnosticReport({
        appVersion: "0.1.0",
        diagnostics,
        generatedAtMs: 1_725_000_001_000,
        platform: "Windows 11 / WebView2",
        runtimeStatus,
      }),
    ) as Record<string, unknown>;

    expect(report).toEqual({
      reportVersion: 1,
      generatedAt: "2024-08-30T06:40:01.000Z",
      appVersion: "0.1.0",
      platform: "Windows 11 / WebView2",
      runtimeStatus,
      diagnostics,
    });
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
  });
});

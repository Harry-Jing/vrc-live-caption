import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { resolveDiagnosticReportHost } from "./diagnosticReportHost";

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

beforeEach(() => {
  platformMocks.getVersion.mockReset();
  platformMocks.isTauri.mockReset();
  platformMocks.writeText.mockReset();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("diagnostic report host", () => {
  test("resolves the native app version and write-only clipboard in Tauri", async () => {
    platformMocks.isTauri.mockReturnValue(true);
    platformMocks.getVersion.mockResolvedValue("0.1.0");
    platformMocks.writeText.mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { userAgent: "Windows / WebView2" });

    const host = await resolveDiagnosticReportHost();
    await host.writeText("report");

    expect(host.appVersion).toBe("0.1.0");
    expect(host.userAgent).toBe("Windows / WebView2");
    expect(platformMocks.writeText).toHaveBeenCalledWith("report");
  });

  test("resolves preview metadata and the browser clipboard outside Tauri", async () => {
    const browserWriteText = vi.fn().mockResolvedValue(undefined);
    platformMocks.isTauri.mockReturnValue(false);
    vi.stubGlobal("navigator", {
      clipboard: { writeText: browserWriteText },
      userAgent: "Browser preview",
    });

    const host = await resolveDiagnosticReportHost();
    await host.writeText("report");

    expect(host.appVersion).toBe("preview");
    expect(host.userAgent).toBe("Browser preview");
    expect(platformMocks.getVersion).not.toHaveBeenCalled();
    expect(platformMocks.writeText).not.toHaveBeenCalled();
    expect(browserWriteText).toHaveBeenCalledWith("report");
  });
});

import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import type { DiagnosticEvent, RuntimeStatusEvent } from "./types";

const DIAGNOSTIC_REPORT_VERSION = 1;

type DiagnosticReportSource = Readonly<{
  diagnostics: readonly DiagnosticEvent[];
  runtimeStatus: RuntimeStatusEvent;
}>;

type DiagnosticReportInput = DiagnosticReportSource &
  Readonly<{
    appVersion: string;
    generatedAtMs: number;
    platform: string;
  }>;

export function serializeDiagnosticReport(input: DiagnosticReportInput) {
  return JSON.stringify(
    {
      reportVersion: DIAGNOSTIC_REPORT_VERSION,
      generatedAt: new Date(input.generatedAtMs).toISOString(),
      appVersion: input.appVersion,
      platform: input.platform,
      runtimeStatus: input.runtimeStatus,
      diagnostics: input.diagnostics,
    },
    null,
    2,
  );
}

export async function copyDiagnosticReport(
  source: DiagnosticReportSource,
  generatedAtMs = Date.now(),
) {
  const runningInTauri = isTauri();
  const report = serializeDiagnosticReport({
    ...source,
    appVersion: runningInTauri ? await getVersion() : "preview",
    generatedAtMs,
    platform: navigator.userAgent,
  });

  if (runningInTauri) {
    await writeText(report);
    return;
  }

  await navigator.clipboard.writeText(report);
}

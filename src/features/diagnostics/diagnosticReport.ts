import { resolveDiagnosticReportHost } from "../../platform/diagnosticReportHost";
import type {
  DiagnosticEvent,
  RuntimeStatusEvent,
} from "../../runtime/runtimeEvents";

const DIAGNOSTIC_REPORT_VERSION = 1;

type DiagnosticReportPlatform = "windows" | "macos" | "linux" | "unknown";

type DiagnosticReportRuntimeStatus = Readonly<
  Pick<RuntimeStatusEvent, "status" | "timestampMs">
>;

type DiagnosticReportEvent = Readonly<
  Pick<DiagnosticEvent, "category" | "severity" | "code" | "timestampMs">
>;

type DiagnosticReportV1 = Readonly<{
  reportVersion: typeof DIAGNOSTIC_REPORT_VERSION;
  generatedAt: string;
  appVersion: string;
  platform: DiagnosticReportPlatform;
  runtimeStatus: DiagnosticReportRuntimeStatus;
  diagnostics: readonly DiagnosticReportEvent[];
}>;

type DiagnosticReportSource = Readonly<{
  diagnostics: readonly DiagnosticEvent[];
  runtimeStatus: RuntimeStatusEvent;
}>;

type DiagnosticReportInput = DiagnosticReportSource &
  Readonly<{
    appVersion: string;
    generatedAtMs: number;
    platform: DiagnosticReportPlatform;
  }>;

function diagnosticReportPlatform(userAgent: string): DiagnosticReportPlatform {
  const normalized = userAgent.toLowerCase();
  if (normalized.includes("windows")) {
    return "windows";
  }
  if (normalized.includes("macintosh") || normalized.includes("mac os")) {
    return "macos";
  }
  if (normalized.includes("linux") || normalized.includes("x11")) {
    return "linux";
  }
  return "unknown";
}

export function serializeDiagnosticReport(input: DiagnosticReportInput) {
  const report = {
    reportVersion: DIAGNOSTIC_REPORT_VERSION,
    generatedAt: new Date(input.generatedAtMs).toISOString(),
    appVersion: input.appVersion,
    platform: input.platform,
    runtimeStatus: {
      status: input.runtimeStatus.status,
      timestampMs: input.runtimeStatus.timestampMs,
    },
    diagnostics: input.diagnostics.map((diagnostic) => ({
      category: diagnostic.category,
      severity: diagnostic.severity,
      code: diagnostic.code,
      timestampMs: diagnostic.timestampMs,
    })),
  } satisfies DiagnosticReportV1;

  return JSON.stringify(report, null, 2);
}

export async function copyDiagnosticReport(
  source: DiagnosticReportSource,
  generatedAtMs = Date.now(),
) {
  const host = await resolveDiagnosticReportHost();
  const report = serializeDiagnosticReport({
    ...source,
    appVersion: host.appVersion,
    generatedAtMs,
    platform: diagnosticReportPlatform(host.userAgent),
  });

  await host.writeText(report);
}

import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { writeText as writeNativeText } from "@tauri-apps/plugin-clipboard-manager";

export type DiagnosticReportHost = Readonly<{
  appVersion: string;
  userAgent: string;
  writeText: (text: string) => Promise<void>;
}>;

export async function resolveDiagnosticReportHost(): Promise<DiagnosticReportHost> {
  const runningInTauri = isTauri();

  return {
    appVersion: runningInTauri ? await getVersion() : "preview",
    userAgent: navigator.userAgent,
    writeText: runningInTauri
      ? writeNativeText
      : (text) => navigator.clipboard.writeText(text),
  };
}

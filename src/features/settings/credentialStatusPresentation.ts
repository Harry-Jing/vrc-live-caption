import { uiText } from "../../i18n/uiText";
import type { CredentialStatus } from "../../runtime/runtimeControl";

export type CredentialStatusPresentation = Readonly<{
  label: string;
  color: "error" | "neutral" | "success";
  failureMessage: string;
  canRemove: boolean;
  isStoredByApp: boolean;
}>;

export function credentialStatusPresentation(
  status: CredentialStatus | null,
): CredentialStatusPresentation {
  if (status === null) {
    return {
      label: uiText("settings.credentials.status.checking"),
      color: "neutral",
      failureMessage: "",
      canRemove: false,
      isStoredByApp: false,
    };
  }

  switch (status.state) {
    case "unconfigured":
      return {
        label: uiText("settings.credentials.status.notSaved"),
        color: "neutral",
        failureMessage: "",
        canRemove: false,
        isStoredByApp: false,
      };
    case "configured":
      return {
        label: uiText(
          status.storage === "environment"
            ? "settings.credentials.status.environment"
            : "settings.credentials.status.system",
          { displaySuffix: status.displaySuffix },
        ),
        color: "success",
        failureMessage: "",
        canRemove: status.storage === "systemCredentialStore",
        isStoredByApp: status.storage === "systemCredentialStore",
      };
    case "unavailable":
      return {
        label: uiText("settings.credentials.status.unavailable"),
        color: "error",
        failureMessage: status.failure.message,
        canRemove: false,
        isStoredByApp: false,
      };
  }
}

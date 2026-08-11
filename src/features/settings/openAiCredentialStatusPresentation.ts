import { uiText } from "../../i18n/uiText";
import type { CredentialStatus } from "../../runtime/runtimeControl";

type OpenAiCredentialStatusPresentation = Readonly<{
  label: string;
  color: "error" | "neutral" | "success";
  failureMessage: string;
  canRemove: boolean;
}>;

export function openAiCredentialStatusPresentation(
  status: CredentialStatus | null,
): OpenAiCredentialStatusPresentation {
  if (status === null) {
    return {
      label: uiText("settings.credentials.openai.status.checking"),
      color: "neutral",
      failureMessage: "",
      canRemove: false,
    };
  }

  switch (status.state) {
    case "unconfigured":
      return {
        label: uiText("settings.credentials.openai.status.notSaved"),
        color: "neutral",
        failureMessage: "",
        canRemove: false,
      };
    case "configured":
      return {
        label: uiText(
          status.storage === "environment"
            ? "settings.credentials.openai.status.environment"
            : "settings.credentials.openai.status.system",
          { displaySuffix: status.displaySuffix },
        ),
        color: "success",
        failureMessage: "",
        canRemove: status.storage === "systemCredentialStore",
      };
    case "unavailable":
      return {
        label: uiText("settings.credentials.openai.status.unavailable"),
        color: "error",
        failureMessage: status.failure.message,
        canRemove: false,
      };
  }
}

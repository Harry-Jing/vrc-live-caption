import {
  appConfigValidationError,
  translationApiBaseUrlValidationReason,
  type AppConfig,
  type TranslationConfig,
  type TranslationEndpoint,
  type TranslationApiBaseUrlValidationReason,
} from "../../runtime/appConfig";
import {
  TRANSLATION_PATHS,
  type ContentSelection,
  type TranslationTarget,
} from "../../runtime/captionPipeline";

export type TranslationSettingsDraft = {
  content: ContentSelection;
  target: TranslationTarget | null;
  endpointKind: TranslationEndpoint["kind"];
  customApiBaseUrl: string;
};

export type TranslationSettingsValidation = Readonly<{
  targetRequired: boolean;
  customApiBaseUrlError: TranslationApiBaseUrlError | null;
  isValid: boolean;
}>;

export type TranslationApiBaseUrlError = TranslationApiBaseUrlValidationReason;

export function createTranslationSettingsDraft(
  config: AppConfig,
): TranslationSettingsDraft {
  const translation = config.translation;

  return {
    content: config.publication.content,
    target: translation?.target ?? null,
    endpointKind: translation?.endpoint.kind ?? "official",
    customApiBaseUrl:
      translation?.endpoint.kind === "custom"
        ? translation.endpoint.apiBaseUrl
        : "",
  };
}

export function translationSettingsValidation(
  draft: TranslationSettingsDraft,
): TranslationSettingsValidation {
  const targetRequired =
    draft.target === null &&
    (draft.content !== "sourceOnly" || draft.endpointKind === "custom");
  const customApiBaseUrlError =
    draft.endpointKind === "custom"
      ? translationApiBaseUrlValidationReason(draft.customApiBaseUrl.trim())
      : null;

  return {
    targetRequired,
    customApiBaseUrlError,
    isValid: !targetRequired && customApiBaseUrlError === null,
  };
}

export function createAppConfigFromTranslationSettings(
  config: AppConfig,
  draft: TranslationSettingsDraft,
): AppConfig | null {
  if (!translationSettingsValidation(draft).isValid) {
    return null;
  }

  const translation: TranslationConfig | null =
    draft.target === null
      ? null
      : {
          path: config.translation?.path ?? TRANSLATION_PATHS[0],
          target: draft.target,
          endpoint:
            draft.endpointKind === "official"
              ? { kind: "official" }
              : {
                  kind: "custom",
                  apiBaseUrl: draft.customApiBaseUrl.trim(),
                },
        };
  const next: AppConfig = {
    ...config,
    audio: { ...config.audio },
    recognition: {
      ...config.recognition,
      expectedLanguages: [...config.recognition.expectedLanguages],
    },
    translation,
    osc: { ...config.osc },
    publication: { ...config.publication, content: draft.content },
    ui: { ...config.ui },
  };

  return appConfigValidationError(next) === null ? next : null;
}

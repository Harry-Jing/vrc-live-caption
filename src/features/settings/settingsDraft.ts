import { computed, ref, toRaw, watch } from "vue";
import {
  translationApiBaseUrlNewEditValidationReason,
  type AppConfig,
  type TranslationApiBaseUrlValidationReason,
  type TranslationConfig,
} from "../../runtime/appConfig";
import type {
  ContentSelection,
  TranslationTarget,
} from "../../runtime/captionPipeline";

export type TranslationSettingsDraft = {
  target: TranslationTarget | null;
  endpointKind: TranslationConfig["endpoint"]["kind"];
  customApiBaseUrl: string;
};

export type SettingsDraft = Omit<AppConfig, "translation"> & {
  translation: TranslationSettingsDraft | null;
};

export type TranslationDraftIssues = Readonly<{
  target: "required" | null;
  customApiBaseUrl: TranslationApiBaseUrlValidationReason | null;
}>;

function createTranslationDraft(
  translation: TranslationConfig | null,
): TranslationSettingsDraft | null {
  if (translation === null) {
    return null;
  }

  return {
    target: translation.target,
    endpointKind: translation.endpoint.kind,
    customApiBaseUrl:
      translation.endpoint.kind === "custom"
        ? translation.endpoint.apiBaseUrl
        : "",
  };
}

function createDraft(config: AppConfig): SettingsDraft {
  const cloned = structuredClone(toRaw(config));

  return {
    ...cloned,
    translation: createTranslationDraft(cloned.translation),
  };
}

function materializeTranslation(
  translation: TranslationSettingsDraft | null,
): TranslationConfig | null {
  if (translation?.target === null || translation === null) {
    return null;
  }

  if (translation.endpointKind === "custom") {
    if (
      translationApiBaseUrlNewEditValidationReason(
        translation.customApiBaseUrl,
      ) !== null
    ) {
      return null;
    }

    return {
      path: "openai/responses-completed-text",
      target: translation.target,
      endpoint: {
        kind: "custom",
        apiBaseUrl: translation.customApiBaseUrl,
      },
    };
  }

  return {
    path: "openai/responses-completed-text",
    target: translation.target,
    endpoint: { kind: "official" },
  };
}

function configFromDraft(
  draft: SettingsDraft,
  dormantTranslationFallback: TranslationConfig | null,
): AppConfig {
  const materializedTranslation = materializeTranslation(draft.translation);

  return {
    schemaVersion: draft.schemaVersion,
    audio: { inputDeviceId: draft.audio.inputDeviceId },
    recognition: {
      path: draft.recognition.path,
      expectedLanguages: draft.recognition.expectedLanguages.map((language) =>
        language.trim(),
      ),
    },
    translation:
      draft.publication.content === "sourceOnly"
        ? (materializedTranslation ?? dormantTranslationFallback)
        : materializedTranslation,
    osc: {
      host: draft.osc.host.trim(),
      port: draft.osc.port,
      enabled: draft.osc.enabled,
    },
    publication: {
      mode: draft.publication.mode,
      content: draft.publication.content,
    },
    ui: { showOngoingPreview: draft.ui.showOngoingPreview },
  };
}

export function useSettingsDraft(savedConfig: () => AppConfig | null) {
  const draft = ref<SettingsDraft | null>(null);
  let lastSyncedConfigJson: string | null = null;
  let lastSyncedDraftJson: string | null = null;

  watch(
    savedConfig,
    (config) => {
      const configJson = config ? JSON.stringify(config) : null;

      // Runtime-control snapshots can replace the config object when only
      // lifecycle state changed. Preserve edits until saved content changes.
      if (configJson === lastSyncedConfigJson) {
        return;
      }

      lastSyncedConfigJson = configJson;
      draft.value = config ? createDraft(config) : null;
      lastSyncedDraftJson = draft.value ? JSON.stringify(draft.value) : null;
    },
    { immediate: true },
  );

  const normalizedExpectedLanguages = computed(() =>
    (draft.value?.recognition.expectedLanguages ?? []).map((language) =>
      language.trim(),
    ),
  );

  const hasValidExpectedLanguages = computed(() => {
    const languages = normalizedExpectedLanguages.value;
    const normalized = languages.map((language) =>
      language.toLocaleLowerCase("en"),
    );

    return (
      languages.length > 0 &&
      languages.every((language) => language.length > 0) &&
      new Set(normalized).size === languages.length
    );
  });

  const translationIssues = computed<TranslationDraftIssues>(() => {
    const current = draft.value;
    if (current === null || current.publication.content === "sourceOnly") {
      return { target: null, customApiBaseUrl: null };
    }

    const translation = current.translation;

    return {
      target: translation?.target == null ? "required" : null,
      customApiBaseUrl:
        translation?.endpointKind === "custom"
          ? translationApiBaseUrlNewEditValidationReason(
              translation.customApiBaseUrl,
            )
          : null,
    };
  });

  const hasValidTranslationSettings = computed(
    () =>
      translationIssues.value.target === null &&
      translationIssues.value.customApiBaseUrl === null,
  );

  const canSave = computed(
    () =>
      draft.value !== null &&
      hasValidExpectedLanguages.value &&
      hasValidTranslationSettings.value,
  );

  const isDirty = computed(() => {
    if (!draft.value || lastSyncedConfigJson === null) {
      return false;
    }

    const saved = savedConfig();
    if (saved === null) {
      return false;
    }

    if (JSON.stringify(draft.value) === lastSyncedDraftJson) {
      return false;
    }

    // Materialize through the reactive proxy so Vue tracks nested field edits
    // without treating inactive editor-only fields as persisted App Config.
    return (
      JSON.stringify(configFromDraft(draft.value, saved.translation)) !==
      lastSyncedConfigJson
    );
  });

  function ensureTranslationDraft() {
    const current = draft.value;
    if (current === null) {
      return null;
    }

    current.translation ??= {
      target: null,
      endpointKind: "official",
      customApiBaseUrl: "",
    };

    return current.translation;
  }

  function selectContent(content: ContentSelection) {
    const current = draft.value;
    if (current === null) {
      return;
    }

    current.publication.content = content;
    if (content !== "sourceOnly") {
      ensureTranslationDraft();
    }
  }

  function selectTranslationTarget(target: TranslationTarget) {
    const translation = ensureTranslationDraft();
    if (translation !== null) {
      translation.target = target;
    }
  }

  function selectTranslationEndpoint(
    endpointKind: TranslationConfig["endpoint"]["kind"],
  ) {
    const translation = ensureTranslationDraft();
    if (translation !== null) {
      translation.endpointKind = endpointKind;
    }
  }

  function setCustomTranslationApiBaseUrl(apiBaseUrl: string) {
    const translation = ensureTranslationDraft();
    if (translation !== null) {
      translation.customApiBaseUrl = apiBaseUrl;
    }
  }

  function createSaveConfig() {
    const saved = savedConfig();

    if (!draft.value || !saved || !canSave.value) {
      return null;
    }

    const next = configFromDraft(draft.value, saved.translation);
    next.osc.port = Number.isFinite(next.osc.port)
      ? next.osc.port
      : saved.osc.port;

    return next;
  }

  return {
    canSave,
    createSaveConfig,
    draft,
    hasValidExpectedLanguages,
    isDirty,
    selectContent,
    selectTranslationEndpoint,
    selectTranslationTarget,
    setCustomTranslationApiBaseUrl,
    translationIssues,
  };
}

import { computed, ref, toRaw, watch } from "vue";
import type { AppConfig } from "../../runtime/appConfig";
import {
  createAppConfigFromTranslationSettings,
  createTranslationSettingsDraft,
  translationSettingsValidation,
  type TranslationSettingsDraft,
} from "./translationSettingsModel";

export function useSettingsDraft(savedConfig: () => AppConfig | null) {
  const draft = ref<AppConfig | null>(null);
  const translationDraft = ref<TranslationSettingsDraft | null>(null);
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
      draft.value = config ? structuredClone(toRaw(config)) : null;
      translationDraft.value = config
        ? createTranslationSettingsDraft(config)
        : null;
      lastSyncedDraftJson = config
        ? settingsDraftJson(config, createTranslationSettingsDraft(config))
        : null;
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

  const isDirty = computed(() => {
    if (
      !draft.value ||
      !translationDraft.value ||
      lastSyncedDraftJson === null
    ) {
      return false;
    }

    // Serialize through the reactive proxy so Vue tracks nested field edits.
    return (
      settingsDraftJson(draft.value, translationDraft.value) !==
      lastSyncedDraftJson
    );
  });

  const hasValidTranslationSettings = computed(
    () =>
      translationDraft.value !== null &&
      translationSettingsValidation(translationDraft.value).isValid,
  );

  function createSaveConfig() {
    const saved = savedConfig();

    if (
      !draft.value ||
      !translationDraft.value ||
      !saved ||
      !hasValidExpectedLanguages.value ||
      !hasValidTranslationSettings.value
    ) {
      return null;
    }

    const next = structuredClone(toRaw(draft.value));
    next.recognition.expectedLanguages = normalizedExpectedLanguages.value;
    next.osc.host = next.osc.host.trim();
    next.osc.port = Number.isFinite(next.osc.port)
      ? next.osc.port
      : saved.osc.port;

    return createAppConfigFromTranslationSettings(next, translationDraft.value);
  }

  return {
    createSaveConfig,
    draft,
    hasValidExpectedLanguages,
    hasValidTranslationSettings,
    isDirty,
    translationDraft,
  };
}

function settingsDraftJson(
  config: AppConfig,
  translation: TranslationSettingsDraft,
): string {
  return JSON.stringify({ config, translation });
}

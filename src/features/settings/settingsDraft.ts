import { computed, ref, toRaw, watch } from "vue";
import type { AppConfig } from "../../runtime/types";

export function useSettingsDraft(savedConfig: () => AppConfig | null) {
  const draft = ref<AppConfig | null>(null);
  let lastSyncedConfigJson: string | null = null;

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
    },
    { immediate: true },
  );

  const normalizedLanguageHints = computed(() =>
    (draft.value?.stt.languages ?? []).map((language) => language.trim()),
  );

  const hasValidLanguageHints = computed(() => {
    const languages = normalizedLanguageHints.value;
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
    if (!draft.value || lastSyncedConfigJson === null) {
      return false;
    }

    // Serialize through the reactive proxy so Vue tracks nested field edits.
    return JSON.stringify(draft.value) !== lastSyncedConfigJson;
  });

  function createSaveConfig() {
    const saved = savedConfig();

    if (!draft.value || !saved || !hasValidLanguageHints.value) {
      return null;
    }

    const next = structuredClone(toRaw(draft.value));
    next.stt.languages = normalizedLanguageHints.value;
    next.osc.host = next.osc.host.trim();
    next.osc.port = Number.isFinite(next.osc.port)
      ? next.osc.port
      : saved.osc.port;

    return next;
  }

  return {
    createSaveConfig,
    draft,
    hasValidLanguageHints,
    isDirty,
  };
}

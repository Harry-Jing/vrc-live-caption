<script setup lang="ts">
import { computed, ref } from "vue";
import type { UiLocale } from "../../i18n/uiLocale";
import { currentUiLocale } from "../../i18n/uiLocale";
import type {
  CredentialId,
  CredentialStatus,
} from "../../runtime/runtimeControl";
import { translationSettingsValidation } from "./translationSettingsModel";
import type {
  TranslationApiBaseUrlError,
  TranslationSettingsDraft,
} from "./translationSettingsModel";
import { translationSettingsText } from "./translationSettingsText";

const props = withDefaults(
  defineProps<{
    modelValue: TranslationSettingsDraft;
    officialCredentialStatus: CredentialStatus | null;
    customCredentialStatus: CredentialStatus | null;
    credentialFailure?: string;
    customCredentialCaptured?: boolean;
    disabled?: boolean;
    showNextStartDisclosure?: boolean;
    locale?: UiLocale;
  }>(),
  {
    credentialFailure: "",
    customCredentialCaptured: false,
    disabled: false,
    showNextStartDisclosure: false,
    locale: currentUiLocale,
  },
);

const emit = defineEmits<{
  "update:modelValue": [value: TranslationSettingsDraft];
  saveCredential: [id: CredentialId, secret: string];
  deleteCredential: [id: CredentialId];
}>();

const customApiKeyInput = ref("");
const isRemoveCustomCredentialModalOpen = ref(false);

const text = (key: Parameters<typeof translationSettingsText>[1]) =>
  translationSettingsText(props.locale, key);

const validation = computed(() =>
  translationSettingsValidation(props.modelValue),
);
const customUrlErrorTextKeys: Record<
  TranslationApiBaseUrlError,
  Parameters<typeof translationSettingsText>[1]
> = {
  invalidUrl: "customUrlInvalid",
  httpsRequired: "customUrlHttpsRequired",
  hostRequired: "customUrlHostRequired",
  userinfoForbidden: "customUrlUserinfoForbidden",
  queryOrFragmentForbidden: "customUrlQueryOrFragmentForbidden",
  invalidPercentEncoding: "customUrlInvalidPercentEncoding",
  responsesPathForbidden: "customUrlResponsesPathForbidden",
};
const customApiBaseUrlError = computed(() => {
  const reason = validation.value.customApiBaseUrlError;

  return reason === null ? false : text(customUrlErrorTextKeys[reason]);
});
const includesTranslation = computed(
  () => props.modelValue.content !== "sourceOnly",
);
const content = computed({
  get: () => props.modelValue.content,
  set: (value: TranslationSettingsDraft["content"]) => {
    updateDraft({ content: value });
  },
});
const targetModelBinding = computed(() =>
  props.modelValue.target === null
    ? {}
    : { modelValue: props.modelValue.target },
);
const endpointKind = computed({
  get: () => props.modelValue.endpointKind,
  set: (value: TranslationSettingsDraft["endpointKind"]) => {
    updateDraft({ endpointKind: value });
  },
});
const customApiBaseUrl = computed({
  get: () => props.modelValue.customApiBaseUrl,
  set: (value: string) => {
    updateDraft({ customApiBaseUrl: value });
  },
});

const contentItems = computed(() => [
  {
    value: "sourceOnly" as const,
    label: text("sourceOnly"),
    description: text("sourceOnlyDescription"),
  },
  {
    value: "translationOnly" as const,
    label: text("translationOnly"),
    description: text("translationOnlyDescription"),
  },
  {
    value: "bilingual" as const,
    label: text("bilingual"),
    description: text("bilingualDescription"),
  },
]);
const targetItems = computed(() => [
  { value: "en" as const, label: text("targetEnglish") },
  {
    value: "zh-Hans" as const,
    label: text("targetSimplifiedChinese"),
  },
]);
const endpointItems = computed(() => [
  {
    value: "official" as const,
    label: text("endpointOfficial"),
    description: text("endpointOfficialDescription"),
  },
  {
    value: "custom" as const,
    label: text("endpointCustom"),
    description: text("endpointCustomDescription"),
  },
]);

const selectedCredentialStatus = computed(() =>
  props.modelValue.endpointKind === "official"
    ? props.officialCredentialStatus
    : props.customCredentialStatus,
);
const selectedCredentialLabel = computed(() => {
  const status = selectedCredentialStatus.value;
  if (status === null) {
    return text("checking");
  }

  switch (status.state) {
    case "unconfigured":
      return text("notSaved");
    case "configured":
      return status.storage === "environment"
        ? text("savedInEnvironment")
        : text("savedInSystem");
    case "unavailable":
      return text("unavailable");
  }
});
const selectedCredentialColor = computed(() => {
  const state = selectedCredentialStatus.value?.state;

  return state === "configured"
    ? ("success" as const)
    : state === "unavailable"
      ? ("error" as const)
      : ("neutral" as const);
});
const selectedCredentialFailure = computed(() => {
  const status = selectedCredentialStatus.value;

  return status?.state === "unavailable" ? status.failure.message : "";
});
const customCredentialCanRemove = computed(
  () =>
    props.customCredentialStatus?.state === "configured" &&
    props.customCredentialStatus.storage === "systemCredentialStore",
);
const customCredentialIsConfigured = computed(
  () => props.customCredentialStatus?.state === "configured",
);
const canSaveCustomCredential = computed(
  () => !props.disabled && customApiKeyInput.value.trim().length > 0,
);

function updateDraft(patch: Partial<TranslationSettingsDraft>) {
  emit("update:modelValue", { ...props.modelValue, ...patch });
}

function selectTarget(value: "en" | "zh-Hans") {
  updateDraft({ target: value });
}

function saveCustomCredential() {
  if (!canSaveCustomCredential.value) {
    return;
  }

  emit("saveCredential", "customTranslation", customApiKeyInput.value);
  customApiKeyInput.value = "";
}

function confirmDeleteCustomCredential() {
  isRemoveCustomCredentialModalOpen.value = false;
  emit("deleteCredential", "customTranslation");
}
</script>

<template>
  <section class="grid gap-4" data-testid="translation-settings" :lang="locale">
    <div>
      <h3 class="text-sm font-semibold text-highlighted">
        {{ text("title") }}
      </h3>
      <p class="mt-1 text-sm text-muted">{{ text("description") }}</p>
    </div>

    <UFormField :label="text('contentLabel')" name="translationContent">
      <URadioGroup
        v-model="content"
        data-testid="translation-content"
        :disabled="disabled"
        :items="contentItems"
        :legend="text('contentLabel')"
        name="translationContent"
        orientation="horizontal"
        :ui="{ fieldset: 'grid grid-cols-1 sm:grid-cols-3', legend: 'sr-only' }"
        variant="card"
      />
    </UFormField>

    <UAlert
      v-if="!includesTranslation"
      color="neutral"
      icon="i-lucide-moon"
      :title="text('dormantTitle')"
      :description="text('dormantDescription')"
      variant="subtle"
    />
    <UAlert
      v-else
      color="info"
      icon="i-lucide-cloud-upload"
      :description="
        modelValue.endpointKind === 'official'
          ? text('officialUploadDisclosure')
          : text('customUploadDisclosure')
      "
      variant="subtle"
    />

    <UAlert
      v-if="showNextStartDisclosure"
      color="warning"
      icon="i-lucide-clock-3"
      :title="text('nextStartTitle')"
      :description="text('nextStartDescription')"
      variant="subtle"
    />

    <UFormField
      :label="text('targetLabel')"
      :description="text('targetDescription')"
      :error="validation.targetRequired ? text('targetRequired') : false"
      name="translationTarget"
      :required="includesTranslation || validation.targetRequired"
    >
      <URadioGroup
        v-bind="targetModelBinding"
        data-testid="translation-target"
        :disabled="disabled"
        :items="targetItems"
        :legend="text('targetLabel')"
        name="translationTarget"
        orientation="horizontal"
        :required="includesTranslation || validation.targetRequired"
        :ui="{ legend: 'sr-only' }"
        variant="card"
        @update:model-value="selectTarget"
      />
    </UFormField>

    <UFormField :label="text('endpointLabel')" name="translationEndpoint">
      <URadioGroup
        v-model="endpointKind"
        data-testid="translation-endpoint"
        :disabled="disabled"
        :items="endpointItems"
        :legend="text('endpointLabel')"
        name="translationEndpoint"
        orientation="horizontal"
        :ui="{ fieldset: 'grid grid-cols-1 sm:grid-cols-2', legend: 'sr-only' }"
        variant="card"
      />
    </UFormField>

    <UFormField
      v-if="modelValue.endpointKind === 'custom'"
      :label="text('customUrlLabel')"
      :description="text('customUrlDescription')"
      :error="customApiBaseUrlError"
      name="customTranslationApiBaseUrl"
      required
    >
      <UInput
        v-model="customApiBaseUrl"
        autocapitalize="off"
        autocomplete="url"
        class="w-full"
        :disabled="disabled"
        inputmode="url"
        :placeholder="text('customUrlPlaceholder')"
        spellcheck="false"
        type="url"
      />
    </UFormField>

    <div class="grid gap-3 rounded-md border border-default bg-muted/30 p-3">
      <div class="flex items-start justify-between gap-3">
        <div>
          <p class="text-sm font-medium text-highlighted">
            {{
              modelValue.endpointKind === "official"
                ? text("officialCredentialTitle")
                : text("customCredentialTitle")
            }}
          </p>
          <p class="mt-1 text-sm text-muted">
            {{
              modelValue.endpointKind === "official"
                ? text("officialCredentialDescription")
                : text("customCredentialDescription")
            }}
          </p>
        </div>
        <UBadge :color="selectedCredentialColor" variant="subtle">
          {{ selectedCredentialLabel }}
        </UBadge>
      </div>

      <template v-if="modelValue.endpointKind === 'custom'">
        <div
          class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-end"
        >
          <UFormField :label="text('apiKeyLabel')">
            <UInput
              v-model="customApiKeyInput"
              autocapitalize="off"
              autocomplete="off"
              class="w-full"
              :disabled="disabled"
              :placeholder="text('apiKeyPlaceholder')"
              spellcheck="false"
              type="password"
            />
          </UFormField>
          <UButton
            :disabled="!canSaveCustomCredential"
            icon="i-lucide-key-round"
            :label="
              customCredentialIsConfigured
                ? text('replaceKey')
                : text('saveKey')
            "
            type="button"
            variant="subtle"
            @click="saveCustomCredential"
          />
          <UButton
            v-if="customCredentialCanRemove"
            :disabled="disabled"
            color="error"
            icon="i-lucide-trash-2"
            :label="text('removeKey')"
            type="button"
            variant="ghost"
            @click="isRemoveCustomCredentialModalOpen = true"
          />
        </div>
      </template>

      <UAlert
        v-if="modelValue.endpointKind === 'custom' && credentialFailure"
        color="error"
        icon="i-lucide-circle-alert"
        role="alert"
        :title="text('credentialActionFailed')"
        :description="credentialFailure"
        variant="subtle"
      />
      <p
        v-if="selectedCredentialFailure"
        class="text-xs text-error"
        role="alert"
      >
        {{ selectedCredentialFailure }}
      </p>
    </div>

    <UModal
      v-model:open="isRemoveCustomCredentialModalOpen"
      :title="text('removeDialogTitle')"
      :description="
        customCredentialCaptured
          ? text('removeDialogCurrentGenerationDescription')
          : text('removeDialogDescription')
      "
    >
      <template #footer>
        <div class="flex w-full justify-end gap-2">
          <UButton
            color="neutral"
            :label="text('cancel')"
            variant="outline"
            @click="isRemoveCustomCredentialModalOpen = false"
          />
          <UButton
            :disabled="disabled"
            color="error"
            icon="i-lucide-trash-2"
            :label="text('removeKey')"
            @click="confirmDeleteCustomCredential"
          />
        </div>
      </template>
    </UModal>
  </section>
</template>

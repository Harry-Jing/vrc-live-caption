<script setup lang="ts">
import { computed } from "vue";
import { uiText, type UiStaticMessageKey } from "../../i18n/uiText";
import type { TranslationApiBaseUrlValidationReason } from "../../runtime/appConfig";
import type {
  ContentSelection,
  PublicationMode,
  TranslationTarget,
} from "../../runtime/captionPipeline";
import type { CredentialStatus } from "../../runtime/runtimeControl";
import { credentialStatusPresentation } from "./credentialStatusPresentation";
import type {
  TranslationDraftIssues,
  TranslationSettingsDraft,
} from "./settingsDraft";

const props = defineProps<{
  content: ContentSelection;
  customCredentialStatus: CredentialStatus | null;
  disabled: boolean;
  issues: TranslationDraftIssues;
  openAiCredentialStatus: CredentialStatus | null;
  publicationMode: PublicationMode;
  translation: TranslationSettingsDraft | null;
}>();

const emit = defineEmits<{
  selectContent: [content: ContentSelection];
  selectEndpoint: [endpoint: "official" | "custom"];
  selectTarget: [target: TranslationTarget];
  setCustomApiBaseUrl: [apiBaseUrl: string];
  useCompleted: [];
}>();

const contentItems = [
  {
    label: uiText("settings.translation.content.sourceOnly"),
    description: uiText("settings.translation.content.sourceOnly.description"),
    value: "sourceOnly",
  },
  {
    label: uiText("settings.translation.content.translationOnly"),
    description: uiText(
      "settings.translation.content.translationOnly.description",
    ),
    value: "translationOnly",
  },
  {
    label: uiText("settings.translation.content.bilingual"),
    description: uiText("settings.translation.content.bilingual.description"),
    value: "bilingual",
  },
] satisfies ReadonlyArray<{
  label: string;
  description: string;
  value: ContentSelection;
}>;

const targetItems: Array<{
  label: string;
  value: TranslationTarget | null;
}> = [
  {
    label: uiText("settings.translation.target.en"),
    value: "en",
  },
  {
    label: uiText("settings.translation.target.zhHans"),
    value: "zh-Hans",
  },
];

const endpointItems = [
  {
    label: uiText("settings.translation.endpoint.official"),
    description: uiText("settings.translation.endpoint.official.description"),
    value: "official",
  },
  {
    label: uiText("settings.translation.endpoint.custom"),
    description: uiText("settings.translation.endpoint.custom.description"),
    value: "custom",
  },
] satisfies Array<{
  label: string;
  description: string;
  value: "official" | "custom";
}>;

const customApiBaseUrlErrorMessageKey = {
  invalidUrl: "settings.translation.customApiBaseUrl.error.invalidUrl",
  httpsRequired: "settings.translation.customApiBaseUrl.error.httpsRequired",
  hostRequired: "settings.translation.customApiBaseUrl.error.hostRequired",
  userInformationForbidden:
    "settings.translation.customApiBaseUrl.error.userInformationForbidden",
  queryOrFragmentForbidden:
    "settings.translation.customApiBaseUrl.error.queryOrFragmentForbidden",
  invalidPercentEncoding:
    "settings.translation.customApiBaseUrl.error.invalidPercentEncoding",
  responsesEndpointForbidden:
    "settings.translation.customApiBaseUrl.error.responsesEndpointForbidden",
} satisfies Record<TranslationApiBaseUrlValidationReason, UiStaticMessageKey>;

const isTranslationActive = computed(() => props.content !== "sourceOnly");
const openAiCredentialPresentation = computed(() =>
  credentialStatusPresentation(props.openAiCredentialStatus),
);
const customCredentialPresentation = computed(() =>
  credentialStatusPresentation(props.customCredentialStatus),
);
const customApiBaseUrlError = computed(() => {
  const reason = props.issues.customApiBaseUrl;

  return reason === null
    ? false
    : uiText(customApiBaseUrlErrorMessageKey[reason]);
});

function selectContent(value: ContentSelection) {
  emit("selectContent", value);
}

function selectTarget(value: TranslationTarget | null) {
  if (value !== null) {
    emit("selectTarget", value);
  }
}

function selectEndpoint(value: "official" | "custom") {
  emit("selectEndpoint", value);
}

function setCustomApiBaseUrl(value: string | number) {
  emit("setCustomApiBaseUrl", String(value));
}
</script>

<template>
  <section class="grid gap-4" aria-labelledby="completed-translation-title">
    <div>
      <h3
        id="completed-translation-title"
        class="text-sm font-semibold text-highlighted"
      >
        {{ uiText("settings.translation.title") }}
      </h3>
      <p class="mt-1 text-sm text-muted">
        {{ uiText("settings.translation.description") }}
      </p>
    </div>

    <UFormField
      :label="uiText('settings.translation.content.legend')"
      :description="uiText('settings.translation.content.description')"
    >
      <URadioGroup
        :disabled="disabled"
        :items="contentItems"
        :legend="uiText('settings.translation.content.legend')"
        :model-value="content"
        name="captionContent"
        orientation="horizontal"
        :ui="{
          fieldset: 'flex-wrap',
          item: 'min-w-52 flex-1',
          legend: 'sr-only',
        }"
        variant="card"
        @update:model-value="selectContent"
      />
    </UFormField>

    <UAlert
      v-if="!isTranslationActive"
      color="neutral"
      icon="i-lucide-circle-pause"
      :title="uiText('settings.translation.inactive.title')"
      :description="uiText('settings.translation.inactive.description')"
      variant="subtle"
    />

    <template v-else>
      <UAlert
        v-if="publicationMode === 'live'"
        color="warning"
        icon="i-lucide-triangle-alert"
        :title="uiText('settings.translation.liveIncompatible.title')"
        :description="
          uiText('settings.translation.liveIncompatible.description')
        "
        variant="subtle"
      >
        <template #actions>
          <UButton
            color="neutral"
            :disabled="disabled"
            :label="uiText('settings.translation.liveIncompatible.action')"
            size="xs"
            type="button"
            variant="outline"
            @click="emit('useCompleted')"
          />
        </template>
      </UAlert>

      <p class="text-xs text-muted">
        {{ uiText("settings.translation.path") }}
      </p>

      <UFormField
        :label="uiText('settings.translation.target.legend')"
        :description="uiText('settings.translation.target.description')"
        :error="
          issues.target === 'required'
            ? uiText('settings.translation.target.required')
            : false
        "
        required
      >
        <URadioGroup
          :disabled="disabled"
          :items="targetItems"
          :legend="uiText('settings.translation.target.legend')"
          :model-value="translation?.target ?? null"
          name="translationTarget"
          orientation="horizontal"
          required
          :ui="{
            fieldset: 'flex-wrap',
            item: 'min-w-44 flex-1',
            legend: 'sr-only',
          }"
          variant="card"
          @update:model-value="selectTarget"
        />
      </UFormField>

      <UFormField :label="uiText('settings.translation.endpoint.legend')">
        <URadioGroup
          :disabled="disabled"
          :items="endpointItems"
          :legend="uiText('settings.translation.endpoint.legend')"
          :model-value="translation?.endpointKind ?? 'official'"
          name="translationEndpoint"
          orientation="horizontal"
          :ui="{
            fieldset: 'flex-wrap',
            item: 'min-w-64 flex-1',
            legend: 'sr-only',
          }"
          variant="card"
          @update:model-value="selectEndpoint"
        />
      </UFormField>

      <template v-if="translation?.endpointKind === 'custom'">
        <UFormField
          :label="uiText('settings.translation.customApiBaseUrl')"
          :description="
            uiText('settings.translation.customApiBaseUrl.description')
          "
          :error="customApiBaseUrlError"
          required
        >
          <UInput
            autocapitalize="off"
            autocomplete="url"
            class="w-full"
            :disabled="disabled"
            :model-value="translation.customApiBaseUrl"
            :placeholder="
              uiText('settings.translation.customApiBaseUrl.placeholder')
            "
            spellcheck="false"
            type="url"
            @update:model-value="setCustomApiBaseUrl"
          />
        </UFormField>

        <UAlert
          color="warning"
          icon="i-lucide-shield-alert"
          :title="uiText('settings.translation.customDisclosure.title')"
          :description="
            uiText('settings.translation.customDisclosure.description')
          "
          variant="subtle"
        />

        <div class="flex items-center justify-between gap-3 text-sm">
          <span class="text-muted">
            {{ uiText("settings.translation.credentialStatus.custom") }}
          </span>
          <UBadge :color="customCredentialPresentation.color" variant="subtle">
            {{ customCredentialPresentation.label }}
          </UBadge>
        </div>
      </template>

      <template v-else>
        <UAlert
          color="info"
          icon="i-lucide-shield-check"
          :title="uiText('settings.translation.officialDisclosure.title')"
          :description="
            uiText('settings.translation.officialDisclosure.description')
          "
          variant="subtle"
        />

        <div class="flex items-center justify-between gap-3 text-sm">
          <span class="text-muted">
            {{ uiText("settings.translation.credentialStatus.openai") }}
          </span>
          <UBadge :color="openAiCredentialPresentation.color" variant="subtle">
            {{ openAiCredentialPresentation.label }}
          </UBadge>
        </div>
      </template>
    </template>

    <p class="text-xs text-muted">
      {{ uiText("settings.translation.nextStart") }}
    </p>
  </section>
</template>

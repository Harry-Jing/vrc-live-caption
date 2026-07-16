<script setup lang="ts">
import { computed, ref, toRaw, watch } from "vue";
import { uiText } from "../i18n/uiText";
import { sttProviderMessageKey } from "../runtime/presentation";
import {
  STT_PROVIDERS,
  type AppConfig,
  type AudioInputDevice,
  type ProviderSecretStatus,
  type SttProvider,
} from "../runtime/types";

const props = defineProps<{
  audioInputDevices: AudioInputDevice[];
  config: AppConfig | null;
  isSecretsBusy: boolean;
  isSettingsBusy: boolean;
  requiresRuntimeRestart: boolean;
  secretStatuses: Partial<Record<SttProvider, ProviderSecretStatus>>;
  secretsError: string;
  settingsError: string;
}>();

const emit = defineEmits<{
  deleteProviderSecret: [provider: SttProvider];
  refreshDevices: [];
  saveConfig: [config: AppConfig];
  saveProviderSecret: [provider: SttProvider, secret: string];
}>();

// The form is a deep clone of the saved config: fields stay editable without
// mutating shared state, and a save round-trip re-syncs it wholesale.
const form = ref<AppConfig | null>(null);
const apiKeyInput = ref("");
const isRemoveKeyModalOpen = ref(false);

watch(
  () => props.config,
  (config) => {
    form.value = config ? structuredClone(toRaw(config)) : null;
  },
  { immediate: true },
);

const openAiSecretStatus = computed(() => props.secretStatuses.openai ?? null);

const canSaveOpenAiApiKey = computed(
  () =>
    form.value?.stt.provider === "openai" &&
    apiKeyInput.value.trim().length > 0,
);

const openAiSecretLabel = computed(() => {
  const status = openAiSecretStatus.value;

  if (!status) {
    return uiText("settings.credentials.openai.status.checking");
  }

  if (!status.configured) {
    return uiText("settings.credentials.openai.status.notSaved");
  }

  if (status.storage === "environment") {
    return uiText("settings.credentials.openai.status.environment", {
      displaySuffix: status.displaySuffix,
    });
  }

  return uiText("settings.credentials.openai.status.system", {
    displaySuffix: status.displaySuffix,
  });
});

const openAiSecretColor = computed<"error" | "neutral" | "success">(() => {
  const status = openAiSecretStatus.value;

  if (status?.error) {
    return "error";
  }

  return status?.configured ? "success" : "neutral";
});

// Sentinel for "use the system default device": the config stores null, but
// reka-ui's Select forbids empty-string item values.
const DEFAULT_DEVICE_VALUE = "__default-input-device__";

const inputDeviceItems = computed(() => {
  const items = [
    {
      label: uiText("audio.devices.defaultInput"),
      value: DEFAULT_DEVICE_VALUE,
    },
    ...props.audioInputDevices.map((device) => ({
      label: device.isDefault
        ? uiText("audio.devices.defaultNamed", { name: device.name })
        : device.name,
      value: device.id,
    })),
  ];
  const selectedId = form.value?.audio.inputDeviceId;

  // Keep a saved-but-disconnected device selectable instead of showing a
  // blank select; the user can keep waiting for it or pick another device.
  if (
    selectedId &&
    !props.audioInputDevices.some((device) => device.id === selectedId)
  ) {
    items.push({
      label: uiText("audio.devices.savedDisconnected"),
      value: selectedId,
    });
  }

  return items;
});

const providerItems = computed(() =>
  STT_PROVIDERS.map((value) => ({
    label: uiText(sttProviderMessageKey[value]),
    value,
  })),
);

const selectedInputDevice = computed({
  get: () => form.value?.audio.inputDeviceId ?? DEFAULT_DEVICE_VALUE,
  set: (value: string) => {
    if (form.value) {
      form.value.audio.inputDeviceId =
        value === DEFAULT_DEVICE_VALUE ? null : value;
    }
  },
});

// Watch the status object, not `.configured`: overwriting an existing key is
// a configured->configured transition, but every save returns a new object.
watch(
  () => openAiSecretStatus.value,
  (status) => {
    if (status?.configured) {
      apiKeyInput.value = "";
    }
  },
);

// UInputNumber yields undefined when cleared; keep the last saved value
// instead of letting backend serde defaults silently replace it.
function finiteOr(value: number, fallback: number) {
  return Number.isFinite(value) ? value : fallback;
}

function save() {
  const saved = props.config;

  if (!form.value || !saved) {
    return;
  }

  const next = structuredClone(toRaw(form.value));
  next.stt.language = next.stt.language.trim();
  next.stt.model = next.stt.model.trim();
  next.osc.host = next.osc.host.trim();
  next.osc.port = finiteOr(next.osc.port, saved.osc.port);

  emit("saveConfig", next);
}

function saveOpenAiApiKey() {
  emit("saveProviderSecret", "openai", apiKeyInput.value);
}

function requestDeleteOpenAiApiKey() {
  isRemoveKeyModalOpen.value = true;
}

function closeRemoveKeyModal() {
  isRemoveKeyModalOpen.value = false;
}

function confirmDeleteOpenAiApiKey() {
  isRemoveKeyModalOpen.value = false;
  emit("deleteProviderSecret", "openai");
}
</script>

<template>
  <UCard :ui="{ body: 'p-5' }">
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-base font-semibold text-highlighted">
            {{ uiText("settings.title") }}
          </h2>
          <p class="mt-1 text-sm text-muted">
            {{ uiText("settings.description") }}
          </p>
        </div>
        <UButton
          :disabled="isSettingsBusy"
          icon="i-lucide-refresh-cw"
          :label="uiText('settings.actions.refreshDevices')"
          size="sm"
          variant="ghost"
          @click="emit('refreshDevices')"
        />
      </div>
    </template>

    <UAlert
      v-if="settingsError"
      class="mb-4"
      color="error"
      icon="i-lucide-circle-alert"
      role="alert"
      :title="uiText('settings.errors.actionFailed')"
      :description="settingsError"
      variant="subtle"
    />

    <UAlert
      v-if="requiresRuntimeRestart"
      class="mb-4"
      color="warning"
      icon="i-lucide-triangle-alert"
      :title="uiText('settings.feedback.restartRequired.title')"
      :description="uiText('settings.feedback.restartRequired.description')"
      variant="subtle"
    />

    <form v-if="form" class="grid gap-5" @submit.prevent="save">
      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">
          {{ uiText("settings.sections.audio") }}
        </h3>

        <UFormField :label="uiText('settings.fields.microphone')">
          <USelect
            v-model="selectedInputDevice"
            class="w-full"
            :items="inputDeviceItems"
          />
        </UFormField>
      </section>

      <USeparator />

      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">
          {{ uiText("settings.sections.speechProvider") }}
        </h3>

        <div class="grid gap-3 sm:grid-cols-2">
          <UFormField :label="uiText('settings.fields.provider')">
            <USelect
              v-model="form.stt.provider"
              class="w-full"
              :items="providerItems"
            />
          </UFormField>

          <UFormField :label="uiText('settings.fields.language')">
            <UInput v-model="form.stt.language" class="w-full" />
          </UFormField>
        </div>

        <UFormField :label="uiText('settings.fields.sttModel')">
          <UInput v-model="form.stt.model" class="w-full" />
        </UFormField>

        <div
          v-if="form.stt.provider === 'openai'"
          class="grid gap-3 rounded-md border border-default bg-muted/30 p-3"
        >
          <div class="flex items-center justify-between gap-3">
            <span class="text-sm font-medium text-highlighted">
              {{ uiText("settings.credentials.openai.title") }}
            </span>
            <UBadge :color="openAiSecretColor" variant="subtle">
              {{ openAiSecretLabel }}
            </UBadge>
          </div>

          <p class="text-sm text-muted">
            {{ uiText("settings.credentials.openai.cloudDisclosure") }}
          </p>

          <div
            class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-end"
          >
            <UFormField :label="uiText('settings.credentials.openai.apiKey')">
              <UInput
                v-model="apiKeyInput"
                autocapitalize="off"
                autocomplete="off"
                class="w-full"
                :placeholder="
                  uiText('settings.credentials.openai.apiKeyPlaceholder')
                "
                spellcheck="false"
                type="password"
              />
            </UFormField>
            <UButton
              :disabled="isSecretsBusy || !canSaveOpenAiApiKey"
              icon="i-lucide-key-round"
              :label="uiText('settings.credentials.openai.actions.save')"
              type="button"
              variant="subtle"
              @click="saveOpenAiApiKey"
            />
            <UButton
              v-if="openAiSecretStatus?.storage === 'systemCredentialStore'"
              :disabled="isSecretsBusy"
              color="error"
              icon="i-lucide-trash-2"
              :label="uiText('settings.credentials.openai.actions.remove')"
              type="button"
              variant="ghost"
              @click="requestDeleteOpenAiApiKey"
            />
          </div>

          <UAlert
            v-if="secretsError"
            color="error"
            icon="i-lucide-circle-alert"
            role="alert"
            :title="uiText('settings.credentials.openai.errors.actionFailed')"
            :description="secretsError"
            variant="subtle"
          />

          <p
            v-if="openAiSecretStatus?.error"
            class="text-xs text-error"
            role="alert"
          >
            {{ openAiSecretStatus.error }}
          </p>
        </div>
      </section>

      <USeparator />

      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">
          {{ uiText("settings.sections.chatboxOutput") }}
        </h3>

        <div class="grid gap-3 sm:grid-cols-[1fr_140px]">
          <UFormField :label="uiText('settings.fields.oscHost')">
            <UInput v-model="form.osc.host" class="w-full" />
          </UFormField>

          <UFormField :label="uiText('settings.fields.port')">
            <UInputNumber
              v-model="form.osc.port"
              class="w-full"
              :format-options="{ useGrouping: false }"
              :max="65535"
              :min="1"
            />
          </UFormField>
        </div>

        <div class="grid gap-3 sm:grid-cols-2">
          <USwitch
            v-model="form.osc.enabled"
            :label="uiText('settings.fields.chatboxOutput')"
          />
          <USwitch
            v-model="form.ui.showPartial"
            :label="uiText('settings.fields.partialPreview')"
          />
        </div>
      </section>

      <UButton
        :disabled="isSettingsBusy"
        icon="i-lucide-save"
        :label="uiText('settings.actions.save')"
        type="submit"
        block
      />
    </form>

    <p v-else class="text-sm text-muted">
      {{
        settingsError
          ? uiText("settings.loadFailed")
          : uiText("settings.loading")
      }}
    </p>
  </UCard>

  <UModal
    v-model:open="isRemoveKeyModalOpen"
    :title="uiText('settings.credentials.openai.removeDialog.title')"
    :description="
      uiText('settings.credentials.openai.removeDialog.description')
    "
  >
    <template #footer>
      <div class="flex w-full justify-end gap-2">
        <UButton
          color="neutral"
          :label="uiText('settings.credentials.openai.removeDialog.cancel')"
          variant="outline"
          @click="closeRemoveKeyModal"
        />
        <UButton
          :disabled="isSecretsBusy"
          color="error"
          icon="i-lucide-trash-2"
          :label="uiText('settings.credentials.openai.removeDialog.confirm')"
          @click="confirmDeleteOpenAiApiKey"
        />
      </div>
    </template>
  </UModal>
</template>

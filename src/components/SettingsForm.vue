<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  toRaw,
  watch,
} from "vue";
import { onBeforeRouteLeave } from "vue-router";
import { uiText } from "../i18n/uiText";
import {
  publicationModeDescriptionMessageKey,
  publicationModeMessageKey,
  publicationPlanDescription,
  publicationSettingsView,
  sttProviderMessageKey,
} from "../runtime/presentation";
import {
  STT_PROVIDERS,
  type AppConfig,
  type AudioInputDevice,
  type PublicationMode,
  type ProviderSecretStatus,
  type RuntimePendingChange,
  type RuntimePlan,
  type RuntimeSessionPhase,
  type SttProvider,
} from "../runtime/types";

const props = defineProps<{
  audioInputDevices: AudioInputDevice[];
  config: AppConfig | null;
  desiredRuntimePlan: RuntimePlan | null;
  isSecretsBusy: boolean;
  isSettingsBusy: boolean;
  pendingSessionChanges: readonly RuntimePendingChange[];
  sessionPhase: RuntimeSessionPhase | null;
  sessionUploadsMicrophoneAudio: boolean;
  secretStatuses: Partial<Record<SttProvider, ProviderSecretStatus>>;
  secretsError: string;
  settingsError: string;
}>();

const emit = defineEmits<{
  deleteProviderSecret: [provider: SttProvider];
  refreshDevices: [];
  saveConfig: [config: AppConfig, onSettled: () => void];
  saveProviderSecret: [provider: SttProvider, secret: string];
}>();

// The form is a deep clone of the saved config: fields stay editable without
// mutating shared state, and a save round-trip re-syncs it wholesale.
const form = ref<AppConfig | null>(null);
const apiKeyInput = ref("");
const isConfigSaveSubmitting = ref(false);
const isRemoveKeyModalOpen = ref(false);
const recognitionFields = ref<HTMLElement | null>(null);
let lastSyncedConfigJson: string | null = null;

watch(
  () => props.config,
  (config) => {
    const configJson = config ? JSON.stringify(config) : null;

    // Full control snapshots may replace the desired config object when only
    // lifecycle state changed. Do not erase an in-progress form draft unless
    // the saved config contents actually changed.
    if (configJson === lastSyncedConfigJson) {
      return;
    }

    lastSyncedConfigJson = configJson;
    form.value = config ? structuredClone(toRaw(config)) : null;
  },
  { immediate: true },
);

const openAiSecretStatus = computed(() => props.secretStatuses.openai ?? null);
const areConfigControlsDisabled = computed(
  () => isConfigSaveSubmitting.value || props.isSettingsBusy,
);

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

const pendingSessionChangesDescription = computed(() => {
  const changes = props.pendingSessionChanges
    .map((change) => {
      switch (change) {
        case "microphone":
          return uiText("settings.feedback.nextStart.change.microphone");
        case "recognition":
          return uiText("settings.feedback.nextStart.change.recognition");
        case "credential":
          return uiText("settings.feedback.nextStart.change.credential");
        case "chatboxOutput":
          return uiText("settings.feedback.nextStart.change.chatboxOutput");
        case "publication":
          return uiText("settings.feedback.nextStart.change.publication");
      }
    })
    .join(", ");

  return uiText(
    props.sessionPhase === "error"
      ? "settings.feedback.nextStart.failedDescription"
      : "settings.feedback.nextStart.description",
    { changes },
  );
});

const removeOpenAiSecretDescription = computed(() =>
  uiText(
    props.sessionUploadsMicrophoneAudio
      ? "settings.credentials.openai.removeDialog.activeSessionDescription"
      : "settings.credentials.openai.removeDialog.description",
  ),
);

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

const PUBLICATION_MODES = [
  "completed",
  "live",
] as const satisfies readonly PublicationMode[];

const publicationModeItems = PUBLICATION_MODES.map((value) => ({
  label: uiText(publicationModeMessageKey[value]),
  description: uiText(publicationModeDescriptionMessageKey[value]),
  value,
}));

const isFormDirty = computed(() => {
  if (!form.value || lastSyncedConfigJson === null) {
    return false;
  }

  // Serialize through the reactive proxy so Vue tracks nested field edits.
  return JSON.stringify(form.value) !== lastSyncedConfigJson;
});

const publicationView = computed(() =>
  publicationSettingsView(props.desiredRuntimePlan, isFormDirty.value),
);

const publicationDescription = computed(() =>
  publicationView.value.state === "ready"
    ? publicationPlanDescription(publicationView.value)
    : "",
);

function confirmDiscardDraft() {
  return (
    !isFormDirty.value ||
    window.confirm(uiText("settings.unsavedChanges.confirmLeave"))
  );
}

function handleBeforeUnload(event: BeforeUnloadEvent) {
  if (!isFormDirty.value) {
    return;
  }

  event.preventDefault();
}

onBeforeRouteLeave(() => confirmDiscardDraft());
onMounted(() => {
  window.addEventListener("beforeunload", handleBeforeUnload);
});
onBeforeUnmount(() => {
  window.removeEventListener("beforeunload", handleBeforeUnload);
});

const selectedInputDevice = computed({
  get: () => form.value?.audio.inputDeviceId ?? DEFAULT_DEVICE_VALUE,
  set: (value: string) => {
    if (form.value) {
      form.value.audio.inputDeviceId =
        value === DEFAULT_DEVICE_VALUE ? null : value;
    }
  },
});

// UInputNumber yields undefined when cleared; keep the last saved value
// instead of letting backend serde defaults silently replace it.
function finiteOr(value: number, fallback: number) {
  return Number.isFinite(value) ? value : fallback;
}

function save() {
  const saved = props.config;

  if (!form.value || !saved || areConfigControlsDisabled.value) {
    return;
  }

  const next = structuredClone(toRaw(form.value));
  next.stt.language = next.stt.language.trim();
  next.stt.model = next.stt.model.trim();
  next.osc.host = next.osc.host.trim();
  next.osc.port = finiteOr(next.osc.port, saved.osc.port);

  isConfigSaveSubmitting.value = true;
  emit("saveConfig", next, () => {
    isConfigSaveSubmitting.value = false;
  });
}

function saveOpenAiApiKey() {
  emit("saveProviderSecret", "openai", apiKeyInput.value);
  // Do not retain plaintext in the form while waiting for secure-store I/O.
  // Full control snapshots are unrelated acknowledgements and must never be
  // used to decide when this local secret input is cleared.
  apiKeyInput.value = "";
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

function selectPublicationMode(mode: PublicationMode) {
  if (form.value) {
    form.value.publication.mode = mode;
  }
}

async function focusRecognitionPath() {
  await nextTick();

  recognitionFields.value?.scrollIntoView({
    behavior: "auto",
    block: "center",
  });
  recognitionFields.value?.querySelector<HTMLElement>("button, input")?.focus();
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
          :disabled="areConfigControlsDisabled"
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
      v-if="pendingSessionChanges.length > 0"
      class="mb-4"
      color="warning"
      icon="i-lucide-triangle-alert"
      :title="uiText('settings.feedback.nextStart.title')"
      :description="pendingSessionChangesDescription"
      variant="subtle"
    />

    <UAlert
      v-if="
        form && sessionUploadsMicrophoneAudio && form.stt.provider !== 'openai'
      "
      class="mb-4"
      color="warning"
      icon="i-lucide-cloud-upload"
      :title="uiText('settings.credentials.openai.activeCloudSession.title')"
      :description="
        uiText('settings.credentials.openai.activeCloudSession.description')
      "
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
            :disabled="areConfigControlsDisabled"
            :items="inputDeviceItems"
          />
        </UFormField>
      </section>

      <USeparator />

      <section ref="recognitionFields" class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">
          {{ uiText("settings.sections.speechProvider") }}
        </h3>

        <div class="grid gap-3 sm:grid-cols-2">
          <UFormField :label="uiText('settings.fields.provider')">
            <USelect
              v-model="form.stt.provider"
              class="w-full"
              :disabled="areConfigControlsDisabled"
              :items="providerItems"
            />
          </UFormField>

          <UFormField :label="uiText('settings.fields.language')">
            <UInput
              v-model="form.stt.language"
              class="w-full"
              :disabled="areConfigControlsDisabled"
            />
          </UFormField>
        </div>

        <UFormField :label="uiText('settings.fields.sttModel')">
          <UInput
            v-model="form.stt.model"
            class="w-full"
            :disabled="areConfigControlsDisabled"
          />
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

        <UFormField
          :label="uiText('settings.fields.publicationMode')"
          :description="uiText('settings.publication.description')"
        >
          <URadioGroup
            v-model="form.publication.mode"
            :disabled="areConfigControlsDisabled"
            :items="publicationModeItems"
            :legend="uiText('settings.fields.publicationMode')"
            name="publicationMode"
            orientation="horizontal"
            :ui="{ legend: 'sr-only' }"
            variant="card"
          />
        </UFormField>

        <p
          v-if="publicationView.state === 'unavailable'"
          class="text-xs text-muted"
        >
          {{ uiText("settings.publication.loading") }}
        </p>

        <UAlert
          v-else-if="publicationView.state === 'unverified'"
          color="info"
          icon="i-lucide-info"
          :title="uiText('settings.publication.unverified.title')"
          :description="uiText('settings.publication.unverified.description')"
          variant="subtle"
        />

        <UAlert
          v-else-if="publicationView.state === 'incompatible'"
          color="error"
          icon="i-lucide-circle-alert"
          role="alert"
          :title="
            uiText('settings.publication.incompatible.title', {
              mode: uiText(publicationModeMessageKey[publicationView.mode]),
            })
          "
          :description="uiText('settings.publication.incompatible.description')"
          variant="subtle"
        >
          <template #actions>
            <UButton
              v-for="mode in publicationView.supportedModes"
              :key="mode"
              color="neutral"
              :label="
                uiText('settings.publication.incompatible.useMode', {
                  mode: uiText(publicationModeMessageKey[mode]),
                })
              "
              size="xs"
              type="button"
              variant="outline"
              @click="selectPublicationMode(mode)"
            />
            <UButton
              color="neutral"
              :label="
                uiText('settings.publication.incompatible.changePath', {
                  mode: uiText(publicationModeMessageKey[publicationView.mode]),
                })
              "
              size="xs"
              type="button"
              variant="outline"
              @click="focusRecognitionPath"
            />
          </template>
        </UAlert>

        <p v-else class="text-xs text-muted">
          {{
            uiText("settings.publication.ready", {
              description: publicationDescription,
            })
          }}
        </p>

        <div class="grid gap-3 sm:grid-cols-[1fr_140px]">
          <UFormField :label="uiText('settings.fields.oscHost')">
            <UInput
              v-model="form.osc.host"
              class="w-full"
              :disabled="areConfigControlsDisabled"
            />
          </UFormField>

          <UFormField :label="uiText('settings.fields.port')">
            <UInputNumber
              v-model="form.osc.port"
              class="w-full"
              :disabled="areConfigControlsDisabled"
              :format-options="{ useGrouping: false }"
              :max="65535"
              :min="1"
            />
          </UFormField>
        </div>

        <div class="grid gap-3 sm:grid-cols-2">
          <USwitch
            v-model="form.osc.enabled"
            :disabled="areConfigControlsDisabled"
            :label="uiText('settings.fields.chatboxOutput')"
          />
          <USwitch
            v-model="form.ui.showPartial"
            :disabled="areConfigControlsDisabled"
            :label="uiText('settings.fields.partialPreview')"
          />
        </div>
      </section>

      <UButton
        :disabled="areConfigControlsDisabled"
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
    :description="removeOpenAiSecretDescription"
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

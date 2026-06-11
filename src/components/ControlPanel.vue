<script setup lang="ts">
import { computed, ref, toRaw, watch } from "vue";
import type {
  AppConfig,
  AudioInputDevice,
  ProviderSecretStatus,
  SttProvider,
} from "../runtime/types";

const props = defineProps<{
  audioInputDevices: AudioInputDevice[];
  config: AppConfig | null;
  isSecretsBusy: boolean;
  isSettingsBusy: boolean;
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
    return "Checking";
  }

  if (!status.configured) {
    return "Not saved";
  }

  const suffix = status.displaySuffix ? `...${status.displaySuffix}` : "saved";

  if (status.storage === "environment") {
    return `Env ${suffix}`;
  }

  return `System ${suffix}`;
});

// Sentinel for "use the system default device": the config stores null, but
// reka-ui's Select forbids empty-string item values.
const DEFAULT_DEVICE_VALUE = "__default-input-device__";

const inputDeviceItems = computed(() => {
  const items = [
    { label: "Default input device", value: DEFAULT_DEVICE_VALUE },
    ...props.audioInputDevices.map((device) => ({
      label: device.isDefault ? `${device.name} (default)` : device.name,
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
    items.push({ label: "Saved device (not connected)", value: selectedId });
  }

  return items;
});

const providerItems: { label: string; value: SttProvider }[] = [
  { label: "OpenAI", value: "openai" },
  { label: "Mock", value: "mock" },
];

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
  next.osc.minIntervalMs = finiteOr(
    next.osc.minIntervalMs,
    saved.osc.minIntervalMs,
  );

  emit("saveConfig", next);
}

function saveOpenAiApiKey() {
  emit("saveProviderSecret", "openai", apiKeyInput.value);
}

function deleteOpenAiApiKey() {
  emit("deleteProviderSecret", "openai");
}
</script>

<template>
  <UCard :ui="{ body: 'p-5' }">
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-base font-semibold text-highlighted">Settings</h2>
          <p class="mt-1 text-sm text-muted">
            Configure capture, provider credentials, and Chatbox output.
          </p>
        </div>
        <UButton
          :disabled="isSettingsBusy"
          icon="i-lucide-refresh-cw"
          label="Devices"
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
      title="Settings action failed"
      :description="settingsError"
      variant="subtle"
    />

    <form v-if="form" class="grid gap-5" @submit.prevent="save">
      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">Audio</h3>

        <UFormField label="Microphone">
          <USelect
            v-model="selectedInputDevice"
            class="w-full"
            :items="inputDeviceItems"
          />
        </UFormField>
      </section>

      <USeparator />

      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">Speech provider</h3>

        <div class="grid gap-3 sm:grid-cols-2">
          <UFormField label="Provider">
            <USelect
              v-model="form.stt.provider"
              class="w-full"
              :items="providerItems"
            />
          </UFormField>

          <UFormField label="Language">
            <UInput v-model="form.stt.language" class="w-full" />
          </UFormField>
        </div>

        <UFormField label="STT model">
          <UInput v-model="form.stt.model" class="w-full" />
        </UFormField>

        <div
          v-if="form.stt.provider === 'openai'"
          class="grid gap-3 rounded-md border border-default bg-muted/30 p-3"
        >
          <div class="flex items-center justify-between gap-3">
            <span class="text-sm font-medium text-highlighted">
              OpenAI API key
            </span>
            <UBadge color="primary" variant="subtle">
              {{ openAiSecretLabel }}
            </UBadge>
          </div>

          <div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto_auto]">
            <UInput
              v-model="apiKeyInput"
              autocapitalize="off"
              autocomplete="off"
              placeholder="sk-..."
              spellcheck="false"
              type="password"
            />
            <UButton
              :disabled="isSecretsBusy || !canSaveOpenAiApiKey"
              icon="i-lucide-key-round"
              label="Save Key"
              type="button"
              variant="subtle"
              @click="saveOpenAiApiKey"
            />
            <UButton
              v-if="openAiSecretStatus?.storage === 'systemCredentialStore'"
              :disabled="isSecretsBusy"
              color="neutral"
              icon="i-lucide-trash-2"
              label="Remove"
              type="button"
              variant="ghost"
              @click="deleteOpenAiApiKey"
            />
          </div>

          <UAlert
            v-if="secretsError"
            color="error"
            icon="i-lucide-circle-alert"
            title="API key action failed"
            :description="secretsError"
            variant="subtle"
          />

          <p v-if="openAiSecretStatus?.error" class="text-xs text-error">
            {{ openAiSecretStatus.error }}
          </p>
        </div>
      </section>

      <USeparator />

      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">Chatbox output</h3>

        <div class="grid gap-3 sm:grid-cols-[1fr_140px]">
          <UFormField label="OSC host">
            <UInput v-model="form.osc.host" class="w-full" />
          </UFormField>

          <UFormField label="Port">
            <UInputNumber
              v-model="form.osc.port"
              class="w-full"
              :format-options="{ useGrouping: false }"
              :max="65535"
              :min="1"
            />
          </UFormField>
        </div>

        <UFormField label="OSC interval (ms)">
          <UInputNumber
            v-model="form.osc.minIntervalMs"
            class="w-full"
            :format-options="{ useGrouping: false }"
            :min="500"
            :step="100"
          />
        </UFormField>

        <div class="grid gap-3 sm:grid-cols-2">
          <USwitch v-model="form.osc.enabled" label="Chatbox output" />
          <USwitch v-model="form.ui.showPartial" label="App partial preview" />
        </div>
      </section>

      <UButton
        :disabled="isSettingsBusy"
        icon="i-lucide-save"
        label="Save Settings"
        type="submit"
        variant="subtle"
        block
      />
    </form>

    <p v-else class="text-sm text-muted">
      {{
        settingsError ? "Settings could not be loaded." : "Loading settings..."
      }}
    </p>
  </UCard>
</template>

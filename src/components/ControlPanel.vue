<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import type {
  AppConfig,
  AudioInputDevice,
  ProviderSecretStatus,
  SttProvider,
} from "../runtime/types";

const props = defineProps<{
  actionError: string;
  audioInputDevices: AudioInputDevice[];
  config: AppConfig | null;
  isBusy: boolean;
  openAiSecretStatus: ProviderSecretStatus | null;
}>();

const emit = defineEmits<{
  deleteProviderSecret: [provider: SttProvider];
  refreshDevices: [];
  saveConfig: [config: AppConfig];
  saveProviderSecret: [provider: SttProvider, secret: string];
}>();

const form = reactive<AppConfig>({
  audio: {
    inputDeviceId: null,
  },
  stt: {
    provider: "openai",
    language: "en",
    model: "gpt-4o-mini-transcribe",
  },
  osc: {
    host: "127.0.0.1",
    port: 9000,
    enabled: true,
    minIntervalMs: 1200,
  },
  ui: {
    showPartial: true,
  },
});
const apiKeyInput = ref("");

const canSaveOpenAiApiKey = computed(
  () => form.stt.provider === "openai" && apiKeyInput.value.trim().length > 0,
);
const openAiSecretLabel = computed(() => {
  const status = props.openAiSecretStatus;

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
const selectedInputDevice = computed({
  get() {
    return form.audio.inputDeviceId ?? "";
  },
  set(value: string) {
    form.audio.inputDeviceId = value || null;
  },
});

watch(
  () => props.config,
  (config) => {
    if (!config) {
      return;
    }

    form.audio.inputDeviceId = config.audio.inputDeviceId;
    form.stt.provider = config.stt.provider;
    form.stt.language = config.stt.language;
    form.stt.model = config.stt.model;
    form.osc.host = config.osc.host;
    form.osc.port = config.osc.port;
    form.osc.enabled = config.osc.enabled;
    form.osc.minIntervalMs = config.osc.minIntervalMs;
    form.ui.showPartial = config.ui.showPartial;
  },
  { immediate: true },
);

watch(
  () => props.openAiSecretStatus,
  (status) => {
    if (status?.configured) {
      apiKeyInput.value = "";
    }
  },
);

function save() {
  emit("saveConfig", {
    audio: {
      inputDeviceId: form.audio.inputDeviceId,
    },
    stt: {
      provider: form.stt.provider,
      language: form.stt.language.trim(),
      model: form.stt.model.trim(),
    },
    osc: {
      host: form.osc.host.trim(),
      port: form.osc.port,
      enabled: form.osc.enabled,
      minIntervalMs: form.osc.minIntervalMs,
    },
    ui: {
      showPartial: form.ui.showPartial,
    },
  });
}

function setProvider(event: Event) {
  form.stt.provider = (event.target as HTMLSelectElement).value as SttProvider;
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
          :disabled="isBusy"
          icon="i-lucide-refresh-cw"
          label="Devices"
          size="sm"
          variant="ghost"
          @click="emit('refreshDevices')"
        />
      </div>
    </template>

    <UAlert
      v-if="actionError"
      class="mb-4"
      color="error"
      icon="i-lucide-circle-alert"
      title="Action failed"
      :description="actionError"
      variant="subtle"
    />

    <form class="grid gap-5" @submit.prevent="save">
      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">Audio</h3>

        <div class="grid gap-2">
          <label class="text-sm font-medium text-highlighted" for="audio-input">
            Microphone
          </label>
          <select
            id="audio-input"
            v-model="selectedInputDevice"
            class="h-10 rounded-md border border-default bg-default px-3 text-sm text-highlighted outline-none focus:border-primary"
          >
            <option value="">Default input device</option>
            <option
              v-for="device in audioInputDevices"
              :key="device.id"
              :value="device.id"
            >
              {{ device.name }}{{ device.isDefault ? " (default)" : "" }}
            </option>
          </select>
        </div>
      </section>

      <USeparator />

      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">Speech provider</h3>

        <div class="grid gap-3 sm:grid-cols-2">
          <div class="grid gap-2">
            <label
              class="text-sm font-medium text-highlighted"
              for="stt-provider"
            >
              Provider
            </label>
            <select
              id="stt-provider"
              :value="form.stt.provider"
              class="h-10 rounded-md border border-default bg-default px-3 text-sm text-highlighted outline-none focus:border-primary"
              @change="setProvider"
            >
              <option value="openai">OpenAI</option>
              <option value="mock">Mock</option>
            </select>
          </div>

          <div class="grid gap-2">
            <label class="text-sm font-medium text-highlighted" for="language">
              Language
            </label>
            <input
              id="language"
              v-model="form.stt.language"
              class="h-10 rounded-md border border-default bg-default px-3 text-sm text-highlighted outline-none focus:border-primary"
              type="text"
            />
          </div>
        </div>

        <div class="grid gap-2">
          <label class="text-sm font-medium text-highlighted" for="stt-model">
            STT model
          </label>
          <input
            id="stt-model"
            v-model="form.stt.model"
            class="h-10 rounded-md border border-default bg-default px-3 text-sm text-highlighted outline-none focus:border-primary"
            type="text"
          />
        </div>

        <div
          v-if="form.stt.provider === 'openai'"
          class="grid gap-3 rounded-md border border-default bg-muted/30 p-3"
        >
          <div class="flex items-center justify-between gap-3">
            <label
              class="text-sm font-medium text-highlighted"
              for="openai-api-key"
            >
              OpenAI API key
            </label>
            <UBadge color="primary" variant="subtle">
              {{ openAiSecretLabel }}
            </UBadge>
          </div>

          <div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto_auto]">
            <input
              id="openai-api-key"
              v-model="apiKeyInput"
              autocomplete="off"
              autocapitalize="off"
              class="h-10 rounded-md border border-default bg-default px-3 text-sm text-highlighted outline-none focus:border-primary"
              inputmode="text"
              placeholder="sk-..."
              spellcheck="false"
              type="password"
            />
            <UButton
              :disabled="isBusy || !canSaveOpenAiApiKey"
              icon="i-lucide-key-round"
              label="Save Key"
              type="button"
              variant="subtle"
              @click="saveOpenAiApiKey"
            />
            <UButton
              v-if="openAiSecretStatus?.storage === 'systemCredentialStore'"
              :disabled="isBusy"
              color="neutral"
              icon="i-lucide-trash-2"
              label="Remove"
              type="button"
              variant="ghost"
              @click="deleteOpenAiApiKey"
            />
          </div>

          <p v-if="openAiSecretStatus?.error" class="text-xs text-error">
            {{ openAiSecretStatus.error }}
          </p>
        </div>
      </section>

      <USeparator />

      <section class="grid gap-4">
        <h3 class="text-sm font-semibold text-highlighted">Chatbox output</h3>

        <div class="grid gap-3 sm:grid-cols-[1fr_96px]">
          <div class="grid gap-2">
            <label class="text-sm font-medium text-highlighted" for="osc-host">
              OSC host
            </label>
            <input
              id="osc-host"
              v-model="form.osc.host"
              class="h-10 rounded-md border border-default bg-default px-3 text-sm text-highlighted outline-none focus:border-primary"
              type="text"
            />
          </div>
          <div class="grid gap-2">
            <label class="text-sm font-medium text-highlighted" for="osc-port">
              Port
            </label>
            <input
              id="osc-port"
              v-model.number="form.osc.port"
              class="h-10 rounded-md border border-default bg-default px-3 text-sm text-highlighted outline-none focus:border-primary"
              min="1"
              max="65535"
              type="number"
            />
          </div>
        </div>

        <div class="grid gap-2">
          <label
            class="text-sm font-medium text-highlighted"
            for="osc-interval"
          >
            OSC interval (ms)
          </label>
          <input
            id="osc-interval"
            v-model.number="form.osc.minIntervalMs"
            class="h-10 rounded-md border border-default bg-default px-3 text-sm text-highlighted outline-none focus:border-primary"
            min="500"
            step="100"
            type="number"
          />
        </div>

        <div class="grid gap-3 sm:grid-cols-2">
          <label class="flex items-center gap-2 text-sm text-highlighted">
            <input
              v-model="form.osc.enabled"
              class="size-4 accent-primary"
              type="checkbox"
            />
            Chatbox output
          </label>
          <label class="flex items-center gap-2 text-sm text-highlighted">
            <input
              v-model="form.ui.showPartial"
              class="size-4 accent-primary"
              type="checkbox"
            />
            App partial preview
          </label>
        </div>
      </section>

      <UButton
        :disabled="isBusy || !config"
        icon="i-lucide-save"
        label="Save Settings"
        type="submit"
        variant="subtle"
        block
      />
    </form>
  </UCard>
</template>

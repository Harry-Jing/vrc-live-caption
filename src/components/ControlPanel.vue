<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import { formatTime } from "../runtime/format";
import type {
  AppConfig,
  AudioInputDevice,
  RuntimeCommand,
  RuntimeStatusEvent,
  SttProvider,
} from "../runtime/types";

const props = defineProps<{
  actionError: string;
  audioInputDevices: AudioInputDevice[];
  config: AppConfig | null;
  isBusy: boolean;
  runtimeStatus: RuntimeStatusEvent;
}>();

const emit = defineEmits<{
  refreshDevices: [];
  run: [command: RuntimeCommand];
  saveConfig: [config: AppConfig];
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

const isRunning = computed(() => props.runtimeStatus.status === "running");
const canStop = computed(() =>
  ["starting", "running", "error"].includes(props.runtimeStatus.status),
);
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

function run(command: RuntimeCommand) {
  emit("run", command);
}

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
</script>

<template>
  <UCard :ui="{ body: 'p-5' }">
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-base font-semibold text-highlighted">
            Outgoing Caption
          </h2>
          <p class="mt-1 text-sm text-muted">
            {{ runtimeStatus.message ?? "No runtime status message." }}
          </p>
        </div>
        <UBadge color="primary" variant="subtle" class="capitalize">
          {{ runtimeStatus.status }}
        </UBadge>
      </div>
    </template>

    <div class="grid gap-3 sm:grid-cols-2">
      <UButton
        :disabled="isBusy || isRunning"
        icon="i-lucide-play"
        label="Start"
        :loading="isBusy && !isRunning"
        block
        @click="run('start_runtime')"
      />
      <UButton
        :disabled="isBusy || !canStop"
        icon="i-lucide-square"
        label="Stop"
        variant="subtle"
        block
        @click="run('stop_runtime')"
      />
      <UButton
        :disabled="isBusy"
        icon="i-lucide-message-square-text"
        label="Mock Transcript"
        variant="subtle"
        block
        @click="run('emit_mock_transcript')"
      />
      <UButton
        :disabled="isBusy"
        icon="i-lucide-radio"
        label="OSC Test"
        variant="subtle"
        block
        @click="run('send_osc_test_message')"
      />
    </div>

    <UAlert
      v-if="actionError"
      class="mt-4"
      color="error"
      icon="i-lucide-circle-alert"
      title="Action failed"
      :description="actionError"
      variant="subtle"
    />

    <USeparator class="my-5" />

    <form class="grid gap-4" @submit.prevent="save">
      <div class="grid gap-2">
        <div class="flex items-center justify-between gap-3">
          <label class="text-sm font-medium text-highlighted" for="audio-input">
            Microphone
          </label>
          <UButton
            :disabled="isBusy"
            icon="i-lucide-refresh-cw"
            size="xs"
            variant="ghost"
            @click="emit('refreshDevices')"
          />
        </div>
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

      <div class="grid gap-3 sm:grid-cols-2">
        <div class="grid gap-2">
          <label
            class="text-sm font-medium text-highlighted"
            for="stt-provider"
          >
            STT provider
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

      <div class="grid gap-2">
        <label class="text-sm font-medium text-highlighted" for="osc-interval">
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

      <UButton
        :disabled="isBusy || !config"
        icon="i-lucide-save"
        label="Save Settings"
        type="submit"
        variant="subtle"
        block
      />
    </form>

    <USeparator class="my-5" />

    <div class="grid gap-3 text-sm">
      <div class="flex items-center justify-between gap-4">
        <span class="text-muted">Last status update</span>
        <span class="font-medium text-highlighted">
          {{ formatTime(runtimeStatus.timestampMs) }}
        </span>
      </div>
      <div class="flex items-center justify-between gap-4">
        <span class="text-muted">Audio input</span>
        <span class="text-right font-medium text-highlighted">
          {{ config?.audio.inputDeviceId ?? "Default device" }}
        </span>
      </div>
      <div class="flex items-center justify-between gap-4">
        <span class="text-muted">OSC target</span>
        <span class="font-medium text-highlighted">
          {{ config ? `${config.osc.host}:${config.osc.port}` : "loading" }}
        </span>
      </div>
    </div>
  </UCard>
</template>

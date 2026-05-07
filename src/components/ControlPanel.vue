<script setup lang="ts">
import { formatTime } from "../runtime/format";
import type {
  AppConfig,
  RuntimeCommand,
  RuntimeStatusEvent,
} from "../runtime/types";

defineProps<{
  actionError: string;
  config: AppConfig | null;
  isBusy: boolean;
  runtimeStatus: RuntimeStatusEvent;
}>();

const emit = defineEmits<{
  run: [command: RuntimeCommand];
}>();

const actions: Array<{
  command: RuntimeCommand;
  icon: string;
  label: string;
  variant?: "solid" | "subtle";
}> = [
  {
    command: "start_mock_runtime",
    icon: "i-lucide-play",
    label: "Start Runtime",
  },
  {
    command: "emit_mock_transcript",
    icon: "i-lucide-message-square-text",
    label: "Mock Transcript",
    variant: "subtle",
  },
  {
    command: "emit_mock_diagnostic",
    icon: "i-lucide-activity",
    label: "Mock Diagnostic",
    variant: "subtle",
  },
  {
    command: "send_osc_test_message",
    icon: "i-lucide-radio",
    label: "OSC Test",
    variant: "subtle",
  },
];

function run(command: RuntimeCommand) {
  emit("run", command);
}
</script>

<template>
  <UCard :ui="{ body: 'p-5' }">
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-base font-semibold text-highlighted">Runtime</h2>
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
        v-for="action in actions"
        :key="action.command"
        :disabled="isBusy"
        :icon="action.icon"
        :label="action.label"
        :loading="isBusy && action.command === 'start_mock_runtime'"
        :variant="action.variant ?? 'solid'"
        block
        @click="run(action.command)"
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
        <span class="text-muted">STT provider</span>
        <span class="font-medium text-highlighted">
          {{ config?.stt.provider ?? "loading" }}
        </span>
      </div>
      <div class="flex items-center justify-between gap-4">
        <span class="text-muted">Language</span>
        <span class="font-medium text-highlighted">
          {{ config?.stt.language ?? "loading" }}
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

<script setup lang="ts">
import { computed } from "vue";
import { uiText } from "../i18n/uiText";
import {
  runtimeStatusColor,
  runtimeStatusMessageKey,
} from "../runtime/presentation";
import type { RuntimeCommand, RuntimeStatusEvent } from "../runtime/types";

const props = defineProps<{
  errorMessage: string;
  isBusy: boolean;
  pendingCommand: RuntimeCommand | null;
  runtimeStatus: RuntimeStatusEvent;
  showMockTranscript: boolean;
}>();

const emit = defineEmits<{
  run: [command: RuntimeCommand];
}>();

const canStart = computed(
  () =>
    !["starting", "running", "stopping"].includes(props.runtimeStatus.status),
);
const canStop = computed(() =>
  ["starting", "running", "error"].includes(props.runtimeStatus.status),
);
const canEmitMockTranscript = computed(
  () => props.runtimeStatus.status === "running",
);

function run(command: RuntimeCommand) {
  emit("run", command);
}
</script>

<template>
  <UCard :ui="{ body: 'p-5' }">
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-base font-semibold text-highlighted">
            {{ uiText("runtime.title") }}
          </h2>
          <p class="mt-1 text-sm text-muted">
            {{ runtimeStatus.message ?? uiText("runtime.status.noMessage") }}
          </p>
        </div>
        <UBadge
          :color="runtimeStatusColor[runtimeStatus.status]"
          variant="subtle"
        >
          {{ uiText(runtimeStatusMessageKey[runtimeStatus.status]) }}
        </UBadge>
      </div>
    </template>

    <div class="grid gap-3 sm:grid-cols-2">
      <UButton
        :disabled="isBusy || !canStart"
        icon="i-lucide-play"
        :label="uiText('runtime.actions.start')"
        :loading="
          pendingCommand === 'start_runtime' ||
          runtimeStatus.status === 'starting'
        "
        block
        @click="run('start_runtime')"
      />
      <UButton
        :disabled="pendingCommand === 'stop_runtime' || !canStop"
        icon="i-lucide-square"
        :label="uiText('runtime.actions.stop')"
        :loading="
          pendingCommand === 'stop_runtime' ||
          runtimeStatus.status === 'stopping'
        "
        variant="subtle"
        block
        @click="run('stop_runtime')"
      />
      <UButton
        v-if="showMockTranscript"
        :disabled="isBusy || !canEmitMockTranscript"
        icon="i-lucide-message-square-text"
        :label="uiText('runtime.actions.mockTranscript')"
        :loading="pendingCommand === 'emit_mock_transcript'"
        variant="subtle"
        block
        @click="run('emit_mock_transcript')"
      />
      <UButton
        :disabled="isBusy"
        icon="i-lucide-radio"
        :label="uiText('runtime.actions.oscTest')"
        :loading="pendingCommand === 'send_osc_test_message'"
        variant="subtle"
        block
        @click="run('send_osc_test_message')"
      />
    </div>

    <UAlert
      v-if="errorMessage"
      class="mt-4"
      color="error"
      icon="i-lucide-circle-alert"
      role="alert"
      :title="uiText('runtime.errors.actionFailed')"
      :description="errorMessage"
      variant="subtle"
    />
  </UCard>
</template>

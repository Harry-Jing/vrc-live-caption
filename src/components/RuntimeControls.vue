<script setup lang="ts">
import { computed } from "vue";
import type { RuntimeCommand, RuntimeStatusEvent } from "../runtime/types";

const props = defineProps<{
  actionError: string;
  isBusy: boolean;
  runtimeStatus: RuntimeStatusEvent;
}>();

const emit = defineEmits<{
  run: [command: RuntimeCommand];
}>();

const isStopping = computed(() => props.runtimeStatus.status === "stopping");
const canStart = computed(
  () =>
    !["starting", "running", "stopping"].includes(props.runtimeStatus.status),
);
const canStop = computed(() =>
  ["starting", "running", "error"].includes(props.runtimeStatus.status),
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
        :disabled="isBusy || !canStart"
        icon="i-lucide-play"
        label="Start"
        :loading="isBusy && canStart"
        block
        @click="run('start_runtime')"
      />
      <UButton
        :disabled="isBusy || !canStop"
        icon="i-lucide-square"
        label="Stop"
        :loading="isStopping"
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
  </UCard>
</template>

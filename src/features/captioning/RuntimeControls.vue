<script setup lang="ts">
import { computed } from "vue";
import { uiText } from "../../i18n/uiText";
import {
  isActiveRuntimeStatus,
  isStoppableRuntimeStatus,
  type RuntimeAction,
} from "../../runtime/lifecycle";
import {
  runtimeStatusColor,
  runtimeStatusMessageKey,
} from "../../runtime/presentation";
import type { RuntimeStatusEvent } from "../../runtime/runtimeEvents";

const props = defineProps<{
  errorMessage: string;
  isBusy: boolean;
  isStartBlocked: boolean;
  inFlightAction: RuntimeAction | null;
  runtimeStatus: RuntimeStatusEvent;
}>();

const emit = defineEmits<{
  run: [action: RuntimeAction];
}>();

const canStart = computed(
  () => !isActiveRuntimeStatus(props.runtimeStatus.status),
);
const canStop = computed(() =>
  isStoppableRuntimeStatus(props.runtimeStatus.status),
);
function run(action: RuntimeAction) {
  emit("run", action);
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
        :disabled="isBusy || isStartBlocked || !canStart"
        icon="i-lucide-play"
        :label="uiText('runtime.actions.start')"
        :loading="
          inFlightAction === 'start' || runtimeStatus.status === 'starting'
        "
        block
        @click="run('start')"
      />
      <UButton
        :disabled="inFlightAction === 'stop' || !canStop"
        icon="i-lucide-square"
        :label="uiText('runtime.actions.stop')"
        :loading="
          inFlightAction === 'stop' || runtimeStatus.status === 'stopping'
        "
        variant="subtle"
        block
        @click="run('stop')"
      />
      <UButton
        :disabled="isBusy"
        icon="i-lucide-radio"
        :label="uiText('runtime.actions.oscTest')"
        :loading="inFlightAction === 'testChatbox'"
        variant="subtle"
        block
        @click="run('testChatbox')"
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

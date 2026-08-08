<script setup lang="ts">
import { computed } from "vue";
import { uiText } from "../i18n/uiText";
import type { AudioProbeResult } from "../runtime/types";

const props = defineProps<{
  disabled: boolean;
  error: string;
  isRunning: boolean;
  result: AudioProbeResult | null;
  runtimeActive: boolean;
}>();

const emit = defineEmits<{
  test: [];
}>();

const isDisabled = computed(
  () => props.disabled || props.runtimeActive || props.isRunning,
);

const resultStatus = computed(() => {
  if (!props.result) {
    return "";
  }
  if (props.result.clipping) {
    return uiText("settings.microphoneTest.clipping");
  }
  return uiText(
    props.result.gateOpen
      ? "settings.microphoneTest.heard"
      : "settings.microphoneTest.belowThreshold",
  );
});

const resultReading = computed(() =>
  props.result
    ? uiText("audio.level.reading", {
        peakDbfs: props.result.peakDbfs,
        rmsDbfs: props.result.rmsDbfs,
      })
    : "",
);
</script>

<template>
  <div class="grid gap-2">
    <div>
      <button
        :aria-busy="isRunning"
        :aria-describedby="
          runtimeActive ? 'microphone-test-runtime-active' : undefined
        "
        class="inline-flex items-center rounded-md border border-default px-3 py-1.5 text-sm font-medium text-highlighted transition-colors hover:bg-muted focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:cursor-not-allowed disabled:opacity-75"
        :disabled="isDisabled"
        type="button"
        @click="emit('test')"
      >
        {{
          uiText(
            isRunning
              ? "settings.microphoneTest.runningAction"
              : "settings.microphoneTest.action",
          )
        }}
      </button>
    </div>

    <p
      v-if="runtimeActive"
      id="microphone-test-runtime-active"
      class="text-xs text-muted"
    >
      {{ uiText("settings.microphoneTest.runtimeActive") }}
    </p>
    <p
      v-else-if="isRunning"
      aria-live="polite"
      class="text-xs text-muted"
      role="status"
    >
      {{ uiText("settings.microphoneTest.pending") }}
    </p>
    <div
      v-else-if="error"
      class="rounded-md border border-error/30 bg-error/5 p-3 text-sm"
      role="alert"
    >
      <p class="font-medium text-error">
        {{ uiText("settings.microphoneTest.errorTitle") }}
      </p>
      <p class="mt-1 text-muted">
        {{ error }}
      </p>
    </div>
    <div
      v-else-if="result"
      aria-live="polite"
      class="rounded-md border border-default bg-muted/30 p-3 text-sm"
      :role="result.clipping ? 'alert' : 'status'"
    >
      <p
        class="font-medium"
        :class="result.clipping ? 'text-error' : 'text-highlighted'"
      >
        {{ resultStatus }}
      </p>
      <p class="mt-1 text-xs text-muted">
        {{ resultReading }}
      </p>
    </div>
  </div>
</template>

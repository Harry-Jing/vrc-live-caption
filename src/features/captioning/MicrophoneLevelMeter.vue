<script setup lang="ts">
import { computed } from "vue";
import { uiText } from "../../i18n/uiText";
import { isActiveRuntimeGenerationPhase } from "../../runtime/lifecycle";
import type { AudioLevelEvent } from "../../runtime/audio";
import type { RuntimeGenerationPhase } from "../../runtime/runtimeControl";

const props = defineProps<{
  generation: number | null;
  level: AudioLevelEvent | null;
  generationPhase: RuntimeGenerationPhase | null;
}>();

const METER_SCALE_MIN_DBFS = -96;
const METER_SCALE_MAX_DBFS = 0;

const isActiveGeneration = computed(() =>
  isActiveRuntimeGenerationPhase(props.generationPhase),
);

const currentLevel = computed(() => {
  if (
    props.generationPhase !== "running" ||
    props.generation === null ||
    props.level?.generation !== props.generation
  ) {
    return null;
  }

  return props.level;
});

function clampDbfs(value: number) {
  return Math.min(METER_SCALE_MAX_DBFS, Math.max(METER_SCALE_MIN_DBFS, value));
}

function meterPercent(value: number) {
  return (
    ((clampDbfs(value) - METER_SCALE_MIN_DBFS) /
      (METER_SCALE_MAX_DBFS - METER_SCALE_MIN_DBFS)) *
    100
  );
}

const rmsDbfs = computed(() =>
  currentLevel.value
    ? clampDbfs(currentLevel.value.rmsDbfs)
    : METER_SCALE_MIN_DBFS,
);
const rmsWidth = computed(() => `${String(meterPercent(rmsDbfs.value))}%`);
const peakPosition = computed(() =>
  currentLevel.value
    ? `${String(meterPercent(currentLevel.value.peakDbfs))}%`
    : "0%",
);

const reading = computed(() => {
  const level = currentLevel.value;
  return level
    ? uiText("audio.level.reading", {
        peakDbfs: level.peakDbfs,
        rmsDbfs: level.rmsDbfs,
      })
    : "";
});

const gateStatus = computed(() => {
  const level = currentLevel.value;
  if (!level) {
    return "";
  }
  return uiText(
    level.gateOpen
      ? "captioning.microphoneMeter.gateOpen"
      : "captioning.microphoneMeter.belowThreshold",
  );
});

const clippingStatus = computed(() =>
  currentLevel.value?.clipping
    ? uiText("captioning.microphoneMeter.clipping")
    : "",
);

const accessibleStatus = computed(() =>
  clippingStatus.value
    ? uiText("captioning.microphoneMeter.accessibleStatuses", {
        clippingStatus: clippingStatus.value,
        gateStatus: gateStatus.value,
      })
    : gateStatus.value,
);

const accessibleValue = computed(() =>
  uiText("captioning.microphoneMeter.accessibleValue", {
    reading: reading.value,
    status: accessibleStatus.value,
  }),
);
</script>

<template>
  <section
    v-if="isActiveGeneration"
    aria-labelledby="live-microphone-meter-title"
    class="grid gap-3 rounded-md border border-default bg-muted/30 p-3"
  >
    <h3
      id="live-microphone-meter-title"
      class="text-sm font-semibold text-highlighted"
    >
      {{ uiText("captioning.microphoneMeter.title") }}
    </h3>

    <p
      v-if="generationPhase === 'reconnecting'"
      aria-live="polite"
      class="text-sm text-muted"
    >
      {{ uiText("captioning.microphoneMeter.reconnecting") }}
    </p>
    <p
      v-else-if="generationPhase === 'stopping'"
      aria-live="polite"
      class="text-sm text-muted"
    >
      {{ uiText("captioning.microphoneMeter.stopping") }}
    </p>
    <p v-else-if="!currentLevel" class="text-sm text-muted">
      {{ uiText("captioning.microphoneMeter.waiting") }}
    </p>

    <div v-else class="grid gap-2">
      <div
        :aria-label="uiText('captioning.microphoneMeter.accessibleLabel')"
        :aria-valuemax="METER_SCALE_MAX_DBFS"
        :aria-valuemin="METER_SCALE_MIN_DBFS"
        :aria-valuenow="rmsDbfs"
        :aria-valuetext="accessibleValue"
        class="relative h-3 overflow-hidden rounded-full bg-default"
        role="progressbar"
      >
        <div
          class="h-full rounded-full transition-[width] duration-100"
          :class="
            currentLevel.clipping
              ? 'bg-error'
              : currentLevel.gateOpen
                ? 'bg-success'
                : 'bg-accented'
          "
          :style="{ width: rmsWidth }"
        />
        <span
          aria-hidden="true"
          class="bg-highlighted absolute inset-y-0 w-0.5"
          :style="{ left: peakPosition }"
        />
      </div>

      <div class="flex flex-wrap items-center justify-between gap-2 text-xs">
        <p class="font-medium text-highlighted">
          {{ reading }}
        </p>
        <div class="flex flex-wrap items-center gap-2">
          <p class="text-muted">
            {{ gateStatus }}
          </p>
          <p v-if="currentLevel.clipping" class="text-error" role="alert">
            {{ clippingStatus }}
          </p>
        </div>
      </div>
    </div>
  </section>
</template>

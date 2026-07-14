<script setup lang="ts">
import { captionModeColor } from "../runtime/presentation";
import type { CaptionMode } from "../runtime/types";

defineProps<{
  hasFinalTranscript: boolean;
  mode: CaptionMode;
  text: string;
}>();
</script>

<template>
  <UCard :ui="{ body: 'p-6 sm:p-7' }">
    <div class="mb-4 flex items-center justify-between gap-4">
      <div>
        <p class="text-xs font-semibold tracking-wide text-muted uppercase">
          Caption Preview
        </p>
        <h2 class="text-base font-semibold text-highlighted">Current output</h2>
      </div>
      <UBadge :color="captionModeColor[mode]" variant="subtle">
        {{ mode }}
      </UBadge>
    </div>

    <p
      class="min-h-24 text-2xl leading-relaxed break-words text-highlighted sm:text-3xl"
    >
      {{ text }}
    </p>
    <p class="sr-only" aria-atomic="true" aria-live="polite">
      {{
        mode === "final" && hasFinalTranscript ? `Final caption: ${text}` : ""
      }}
    </p>
  </UCard>
</template>

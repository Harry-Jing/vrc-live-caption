<script setup lang="ts">
import { uiText } from "../../i18n/uiText";
import {
  captionPreviewStatusColor,
  captionPreviewStatusIcon,
  captionPreviewStatusMessageKey,
} from "../../runtime/presentation";
import type { CaptionPreviewStatus } from "../../runtime/presentation";

defineProps<{
  latestCompletedCaption: Readonly<{ id: string; text: string }> | null;
  status: CaptionPreviewStatus;
  text: string;
}>();
</script>

<template>
  <UCard :ui="{ body: 'p-6 sm:p-7' }">
    <div class="mb-4 flex items-center justify-between gap-4">
      <div>
        <p class="text-xs font-semibold tracking-wide text-muted uppercase">
          {{ uiText("caption.preview.eyebrow") }}
        </p>
        <h2 class="text-base font-semibold text-highlighted">
          {{ uiText("caption.preview.title") }}
        </h2>
      </div>
      <UBadge
        :color="captionPreviewStatusColor[status]"
        aria-atomic="true"
        aria-live="polite"
        role="status"
        variant="subtle"
      >
        <UIcon
          :name="captionPreviewStatusIcon[status]"
          class="size-3.5"
          aria-hidden="true"
        />
        {{ uiText(captionPreviewStatusMessageKey[status]) }}
      </UBadge>
    </div>

    <p
      class="min-h-24 text-2xl leading-relaxed break-words text-highlighted sm:text-3xl"
    >
      {{ text }}
    </p>
    <p class="sr-only" aria-atomic="true" aria-live="polite">
      <span v-if="latestCompletedCaption" :key="latestCompletedCaption.id">
        {{
          uiText("caption.completedAnnouncement", {
            text: latestCompletedCaption.text,
          })
        }}
      </span>
    </p>
  </UCard>
</template>

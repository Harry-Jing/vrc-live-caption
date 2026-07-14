<script setup lang="ts">
import EventFeed from "../components/EventFeed.vue";
import { uiText } from "../i18n/uiText";
import { formatTime } from "../runtime/format";
import { useRuntimeContext } from "../runtime/context";
import {
  runtimeStatusColor,
  runtimeStatusMessageKey,
} from "../runtime/presentation";

const { diagnostics, finalTranscripts, runtimeStatus } = useRuntimeContext();
</script>

<template>
  <div class="grid gap-5">
    <header
      class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between"
    >
      <div>
        <p class="text-xs font-semibold tracking-wide text-muted uppercase">
          {{ uiText("diagnostics.title") }}
        </p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight text-highlighted">
          {{ uiText("diagnostics.page.title") }}
        </h1>
      </div>

      <div class="flex flex-wrap gap-2">
        <UBadge
          :color="runtimeStatusColor[runtimeStatus.status]"
          variant="subtle"
        >
          {{ uiText(runtimeStatusMessageKey[runtimeStatus.status]) }}
        </UBadge>
        <UBadge color="neutral" variant="subtle">
          {{ formatTime(runtimeStatus.timestampMs) }}
        </UBadge>
      </div>
    </header>

    <EventFeed
      :diagnostics="diagnostics"
      :final-transcripts="finalTranscripts"
    />
  </div>
</template>

<script setup lang="ts">
import EventFeed from "../components/EventFeed.vue";
import { formatTime } from "../runtime/format";
import { runtimeStatusColor } from "../runtime/presentation";
import { useRuntimeContext } from "../runtime/context";

const { diagnostics, finalTranscripts, runtimeStatus } = useRuntimeContext();
</script>

<template>
  <div class="grid gap-5">
    <header
      class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between"
    >
      <div>
        <p class="text-xs font-semibold tracking-wide text-muted uppercase">
          Diagnostics
        </p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight text-highlighted">
          Runtime events and transcripts
        </h1>
      </div>

      <div class="flex flex-wrap gap-2">
        <UBadge
          :color="runtimeStatusColor[runtimeStatus.status]"
          variant="subtle"
          class="capitalize"
        >
          {{ runtimeStatus.status }}
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

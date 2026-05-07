<script setup lang="ts">
import { formatTime } from "../runtime/format";
import type { DiagnosticEvent, TranscriptEvent } from "../runtime/types";

defineProps<{
  diagnostics: DiagnosticEvent[];
  finalTranscripts: TranscriptEvent[];
}>();

function diagnosticColor(severity: DiagnosticEvent["severity"]) {
  if (severity === "error") {
    return "error";
  }

  if (severity === "warning") {
    return "warning";
  }

  return "primary";
}
</script>

<template>
  <div class="grid gap-4">
    <UCard :ui="{ body: 'p-5' }">
      <template #header>
        <div class="flex items-center justify-between gap-4">
          <h2 class="text-base font-semibold text-highlighted">Diagnostics</h2>
          <UBadge color="neutral" variant="subtle">{{
            diagnostics.length
          }}</UBadge>
        </div>
      </template>

      <ol v-if="diagnostics.length" class="grid gap-3">
        <li
          v-for="diagnostic in diagnostics"
          :key="diagnostic.id"
          class="rounded-md border border-default bg-muted/40 p-3"
        >
          <div class="flex items-start justify-between gap-3">
            <div>
              <p class="font-medium text-highlighted">
                {{ diagnostic.message }}
              </p>
              <p v-if="diagnostic.detail" class="mt-1 text-sm text-muted">
                {{ diagnostic.detail }}
              </p>
            </div>
            <UBadge
              :color="diagnosticColor(diagnostic.severity)"
              variant="subtle"
            >
              {{ diagnostic.category }}
            </UBadge>
          </div>
          <p class="mt-2 text-xs text-muted">
            {{ formatTime(diagnostic.timestampMs) }}
          </p>
        </li>
      </ol>

      <p v-else class="text-sm text-muted">No diagnostics yet.</p>
    </UCard>

    <UCard :ui="{ body: 'p-5' }">
      <template #header>
        <div class="flex items-center justify-between gap-4">
          <h2 class="text-base font-semibold text-highlighted">
            Final Transcripts
          </h2>
          <UBadge color="neutral" variant="subtle">{{
            finalTranscripts.length
          }}</UBadge>
        </div>
      </template>

      <ol v-if="finalTranscripts.length" class="grid gap-3">
        <li
          v-for="transcript in finalTranscripts"
          :key="transcript.id"
          class="grid gap-1 rounded-md border border-default bg-muted/40 p-3"
        >
          <span class="text-xs text-muted">{{
            formatTime(transcript.timestampMs)
          }}</span>
          <p class="text-sm leading-6 text-highlighted">
            {{ transcript.text }}
          </p>
        </li>
      </ol>

      <p v-else class="text-sm text-muted">No final transcript events yet.</p>
    </UCard>
  </div>
</template>

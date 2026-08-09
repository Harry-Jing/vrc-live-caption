<script setup lang="ts">
import { uiText } from "../i18n/uiText";
import { formatTime } from "../runtime/format";
import {
  diagnosticCategoryMessageKey,
  diagnosticSeverityColor,
  diagnosticSeverityMessageKey,
} from "../runtime/presentation";
import type { CaptionDisplay, DiagnosticEvent } from "../runtime/types";

defineProps<{
  diagnostics: readonly DiagnosticEvent[];
  completedCaptions: readonly CaptionDisplay[];
}>();
</script>

<template>
  <div class="grid gap-4">
    <UCard :ui="{ body: 'p-5' }">
      <template #header>
        <div class="flex items-center justify-between gap-4">
          <h2 class="text-base font-semibold text-highlighted">
            {{ uiText("diagnostics.title") }}
          </h2>
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
            <div class="min-w-0">
              <p class="font-medium break-words text-highlighted">
                {{ diagnostic.message }}
              </p>
              <p
                v-if="diagnostic.detail"
                class="mt-1 text-sm break-words text-muted"
              >
                {{ diagnostic.detail }}
              </p>
            </div>
            <UBadge
              :color="diagnosticSeverityColor[diagnostic.severity]"
              class="shrink-0"
              variant="subtle"
            >
              {{ uiText(diagnosticSeverityMessageKey[diagnostic.severity]) }} ·
              {{ uiText(diagnosticCategoryMessageKey[diagnostic.category]) }}
            </UBadge>
          </div>
          <p class="mt-2 text-xs text-muted">
            {{ formatTime(diagnostic.timestampMs) }}
          </p>
        </li>
      </ol>

      <p v-else class="text-sm text-muted">
        {{ uiText("diagnostics.empty") }}
      </p>
    </UCard>

    <UCard :ui="{ body: 'p-5' }">
      <template #header>
        <div class="flex items-center justify-between gap-4">
          <h2 class="text-base font-semibold text-highlighted">
            {{ uiText("diagnostics.completedCaptions.title") }}
          </h2>
          <UBadge color="neutral" variant="subtle">{{
            completedCaptions.length
          }}</UBadge>
        </div>
      </template>

      <ol v-if="completedCaptions.length" class="grid gap-3">
        <li
          v-for="caption in completedCaptions"
          :key="caption.id"
          class="grid gap-1 rounded-md border border-default bg-muted/40 p-3"
        >
          <span class="text-xs text-muted">{{
            formatTime(caption.timestampMs)
          }}</span>
          <p class="text-sm leading-6 break-words text-highlighted">
            {{ caption.text }}
          </p>
        </li>
      </ol>

      <p v-else class="text-sm text-muted">
        {{ uiText("diagnostics.completedCaptions.empty") }}
      </p>
    </UCard>
  </div>
</template>

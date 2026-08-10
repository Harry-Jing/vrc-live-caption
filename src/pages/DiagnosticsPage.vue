<script setup lang="ts">
import { computed, ref } from "vue";
import EventFeed from "../components/EventFeed.vue";
import { uiText } from "../i18n/uiText";
import { copyDiagnosticReport } from "../platform/diagnosticReport";
import { formatTime } from "../runtime/format";
import { useRuntimeContext } from "../runtime/context";
import {
  runtimeStatusColor,
  runtimeStatusMessageKey,
} from "../runtime/presentation";

const { completedCaptions, diagnostics, runtimeStatus } = useRuntimeContext();

const reportCopyState = ref<"idle" | "copying" | "copied" | "failed">("idle");
const reportCopyLabel = computed(() => {
  switch (reportCopyState.value) {
    case "copying":
      return uiText("diagnostics.report.copying");
    case "copied":
      return uiText("diagnostics.report.copied");
    case "failed":
      return uiText("diagnostics.report.copyFailed");
    case "idle":
      return uiText("diagnostics.report.copy");
  }
});

async function copyReport() {
  reportCopyState.value = "copying";
  try {
    await copyDiagnosticReport({
      diagnostics: diagnostics.value,
      runtimeStatus: runtimeStatus.value,
    });
    reportCopyState.value = "copied";
  } catch (error) {
    console.error("Failed to copy the diagnostic report.", error);
    reportCopyState.value = "failed";
  }
}
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

      <div class="flex flex-wrap items-center justify-end gap-2">
        <UButton
          color="neutral"
          :loading="reportCopyState === 'copying'"
          variant="outline"
          @click="copyReport"
        >
          {{ reportCopyLabel }}
        </UButton>
        <UBadge
          :color="runtimeStatusColor[runtimeStatus.status]"
          variant="subtle"
        >
          {{ uiText(runtimeStatusMessageKey[runtimeStatus.status]) }}
        </UBadge>
        <UBadge color="neutral" variant="subtle">
          {{ formatTime(runtimeStatus.timestampMs) }}
        </UBadge>
        <span class="sr-only" aria-live="polite">{{ reportCopyLabel }}</span>
      </div>
    </header>

    <EventFeed
      :completed-captions="completedCaptions"
      :diagnostics="diagnostics"
    />
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { RouterLink } from "vue-router";
import CaptionPreview from "../components/CaptionPreview.vue";
import RuntimeControls from "../components/RuntimeControls.vue";
import { formatTime } from "../runtime/format";
import { useRuntimeContext } from "../runtime/context";

const {
  actionError,
  activeCaptionText,
  captionMode,
  config,
  diagnostics,
  finalTranscripts,
  isBusy,
  runCommand,
  runtimeStatus,
} = useRuntimeContext();

const latestDiagnostic = computed(() => diagnostics.value.at(0) ?? null);
const latestFinalTranscript = computed(
  () => finalTranscripts.value.at(0) ?? null,
);
</script>

<template>
  <div class="grid gap-5">
    <header
      class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between"
    >
      <div>
        <p class="text-xs font-semibold tracking-wide text-muted uppercase">
          Live caption
        </p>
        <h2 class="mt-1 text-2xl font-semibold tracking-tight text-highlighted">
          Speak, preview, send final text
        </h2>
      </div>

      <div class="flex flex-wrap gap-2">
        <UBadge color="neutral" variant="subtle">
          {{ config?.stt.provider ?? "loading" }}
        </UBadge>
        <UBadge
          :color="config?.osc.enabled ? 'success' : 'neutral'"
          variant="subtle"
        >
          {{ config?.osc.enabled ? "Chatbox on" : "Chatbox off" }}
        </UBadge>
      </div>
    </header>

    <CaptionPreview :mode="captionMode" :text="activeCaptionText" />

    <div class="grid gap-5 xl:grid-cols-[minmax(0,1fr)_340px]">
      <RuntimeControls
        :action-error="actionError"
        :is-busy="isBusy"
        :runtime-status="runtimeStatus"
        @run="runCommand"
      />

      <div class="grid gap-5">
        <UCard :ui="{ body: 'p-5' }">
          <template #header>
            <div class="flex items-center justify-between gap-4">
              <h3 class="text-base font-semibold text-highlighted">
                Current setup
              </h3>
              <RouterLink
                class="text-sm font-medium text-primary outline-none hover:text-primary-600 focus-visible:ring-2 focus-visible:ring-primary"
                to="/settings"
              >
                Edit
              </RouterLink>
            </div>
          </template>

          <dl class="grid gap-3 text-sm">
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">Microphone</dt>
              <dd class="text-right font-medium text-highlighted">
                {{ config?.audio.inputDeviceId ?? "Default device" }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">STT model</dt>
              <dd class="text-right font-medium text-highlighted">
                {{ config?.stt.model ?? "loading" }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">OSC target</dt>
              <dd class="font-medium text-highlighted">
                {{
                  config ? `${config.osc.host}:${config.osc.port}` : "loading"
                }}
              </dd>
            </div>
          </dl>
        </UCard>

        <UCard :ui="{ body: 'p-5' }">
          <template #header>
            <div class="flex items-center justify-between gap-4">
              <h3 class="text-base font-semibold text-highlighted">
                Recent activity
              </h3>
              <RouterLink
                class="text-sm font-medium text-primary outline-none hover:text-primary-600 focus-visible:ring-2 focus-visible:ring-primary"
                to="/diagnostics"
              >
                Open
              </RouterLink>
            </div>
          </template>

          <div class="grid gap-4 text-sm">
            <div>
              <p class="text-muted">Latest final transcript</p>
              <p class="mt-1 leading-6 text-highlighted">
                {{ latestFinalTranscript?.text ?? "No final transcript yet." }}
              </p>
            </div>

            <USeparator />

            <div>
              <p class="text-muted">Latest diagnostic</p>
              <p class="mt-1 font-medium text-highlighted">
                {{ latestDiagnostic?.message ?? "No diagnostics yet." }}
              </p>
              <p v-if="latestDiagnostic" class="mt-1 text-xs text-muted">
                {{ formatTime(latestDiagnostic.timestampMs) }}
              </p>
            </div>
          </div>
        </UCard>
      </div>
    </div>
  </div>
</template>

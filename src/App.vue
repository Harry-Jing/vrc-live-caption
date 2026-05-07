<script setup lang="ts">
import { computed } from "vue";
import CaptionPreview from "./components/CaptionPreview.vue";
import ControlPanel from "./components/ControlPanel.vue";
import EventFeed from "./components/EventFeed.vue";
import { useRuntime } from "./runtime/useRuntime";

const {
  actionError,
  activeCaptionText,
  config,
  diagnostics,
  finalTranscripts,
  isBusy,
  partialTranscript,
  runCommand,
  runtimeStatus,
} = useRuntime();

const captionMode = computed(() =>
  partialTranscript.value ? "partial" : "final",
);
</script>

<template>
  <UApp>
    <main
      class="min-h-dvh bg-slate-100 text-slate-950 dark:bg-slate-950 dark:text-slate-50"
    >
      <div
        class="mx-auto grid w-full max-w-6xl gap-5 px-4 py-5 sm:px-6 lg:px-8"
      >
        <header
          class="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between"
        >
          <div>
            <p class="text-xs font-semibold tracking-wide text-muted uppercase">
              VRC Live Caption
            </p>
            <h1
              class="mt-1 text-2xl font-semibold tracking-tight text-highlighted"
            >
              Runtime Foundation
            </h1>
          </div>

          <UBadge
            color="primary"
            icon="i-lucide-radio-tower"
            size="lg"
            variant="subtle"
            class="w-fit capitalize"
          >
            {{ runtimeStatus.status }}
          </UBadge>
        </header>

        <CaptionPreview :mode="captionMode" :text="activeCaptionText" />

        <div
          class="grid gap-5 lg:grid-cols-[minmax(320px,0.82fr)_minmax(0,1.18fr)]"
        >
          <ControlPanel
            :action-error="actionError"
            :config="config"
            :is-busy="isBusy"
            :runtime-status="runtimeStatus"
            @run="runCommand"
          />
          <EventFeed
            :diagnostics="diagnostics"
            :final-transcripts="finalTranscripts"
          />
        </div>
      </div>
    </main>
  </UApp>
</template>

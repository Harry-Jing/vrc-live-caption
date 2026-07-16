<script setup lang="ts">
import { computed } from "vue";
import CaptionPreview from "../components/CaptionPreview.vue";
import RuntimeControls from "../components/RuntimeControls.vue";
import { uiText } from "../i18n/uiText";
import { formatTime } from "../runtime/format";
import { useRuntimeContext } from "../runtime/context";
import { sttProviderMessageKey } from "../runtime/presentation";

const {
  activeCaptionText,
  audioInputDevices,
  captionMode,
  config,
  diagnostics,
  finalTranscripts,
  isRuntimeBusy,
  pendingRuntimeCommand,
  runCommand,
  runtimeError,
  runtimeStatus,
} = useRuntimeContext();

const latestDiagnostic = computed(() => diagnostics.value.at(0) ?? null);
const latestFinalTranscript = computed(
  () => finalTranscripts.value.at(0) ?? null,
);

const currentMicrophoneLabel = computed(() => {
  const currentConfig = config.value;

  if (!currentConfig) {
    return uiText("common.loading");
  }

  const selectedId = currentConfig.audio.inputDeviceId;

  if (!selectedId) {
    const defaultDevice = audioInputDevices.value.find(
      (device) => device.isDefault,
    );

    return defaultDevice
      ? uiText("audio.devices.defaultNamed", { name: defaultDevice.name })
      : uiText("audio.devices.default");
  }

  return (
    audioInputDevices.value.find((device) => device.id === selectedId)?.name ??
    uiText("audio.devices.savedDisconnected")
  );
});
</script>

<template>
  <div class="grid gap-5">
    <header
      class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between"
    >
      <div>
        <p class="text-xs font-semibold tracking-wide text-muted uppercase">
          {{ uiText("live.eyebrow") }}
        </p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight text-highlighted">
          {{ uiText("live.title") }}
        </h1>
      </div>

      <div class="flex flex-wrap gap-2">
        <UBadge color="neutral" variant="subtle">
          {{
            config
              ? uiText(sttProviderMessageKey[config.stt.provider])
              : uiText("common.loading")
          }}
        </UBadge>
        <UBadge
          :color="config?.osc.enabled ? 'success' : 'neutral'"
          variant="subtle"
        >
          {{
            uiText(config?.osc.enabled ? "live.chatbox.on" : "live.chatbox.off")
          }}
        </UBadge>
      </div>
    </header>

    <CaptionPreview
      :latest-final-transcript="latestFinalTranscript"
      :mode="captionMode"
      :text="activeCaptionText"
    />

    <div class="grid gap-5 xl:grid-cols-[minmax(0,1fr)_340px]">
      <RuntimeControls
        :error-message="runtimeError"
        :is-busy="isRuntimeBusy"
        :pending-command="pendingRuntimeCommand"
        :runtime-status="runtimeStatus"
        :show-mock-transcript="config?.stt.provider === 'mock'"
        @run="runCommand"
      />

      <div class="grid gap-5">
        <UCard :ui="{ body: 'p-5' }">
          <template #header>
            <div class="flex items-center justify-between gap-4">
              <h2 class="text-base font-semibold text-highlighted">
                {{ uiText("live.currentSetup.title") }}
              </h2>
              <UButton
                :label="uiText('live.currentSetup.edit')"
                size="sm"
                to="/settings"
                variant="link"
              />
            </div>
          </template>

          <dl class="grid gap-3 text-sm">
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("live.currentSetup.microphone") }}
              </dt>
              <dd
                class="min-w-0 text-right font-medium break-words text-highlighted"
              >
                {{ currentMicrophoneLabel }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("live.currentSetup.sttModel") }}
              </dt>
              <dd class="text-right font-medium text-highlighted">
                {{ config?.stt.model ?? uiText("common.loading") }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("live.currentSetup.oscTarget") }}
              </dt>
              <dd class="font-medium text-highlighted">
                {{
                  config
                    ? `${config.osc.host}:${config.osc.port}`
                    : uiText("common.loading")
                }}
              </dd>
            </div>
          </dl>
        </UCard>

        <UCard :ui="{ body: 'p-5' }">
          <template #header>
            <div class="flex items-center justify-between gap-4">
              <h2 class="text-base font-semibold text-highlighted">
                {{ uiText("live.recentActivity.title") }}
              </h2>
              <UButton
                :label="uiText('live.recentActivity.open')"
                size="sm"
                to="/diagnostics"
                variant="link"
              />
            </div>
          </template>

          <div class="grid gap-4 text-sm">
            <div>
              <p class="text-muted">
                {{ uiText("live.recentActivity.latestFinalTranscript") }}
              </p>
              <p class="mt-1 leading-6 break-words text-highlighted">
                {{
                  latestFinalTranscript?.text ??
                  uiText("live.recentActivity.noFinalTranscript")
                }}
              </p>
            </div>

            <USeparator />

            <div>
              <p class="text-muted">
                {{ uiText("live.recentActivity.latestDiagnostic") }}
              </p>
              <p class="mt-1 font-medium text-highlighted">
                {{ latestDiagnostic?.message ?? uiText("diagnostics.empty") }}
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

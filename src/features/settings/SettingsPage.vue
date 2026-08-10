<script setup lang="ts">
import { useToast } from "@nuxt/ui/composables";
import { computed } from "vue";
import SettingsForm from "./SettingsForm.vue";
import { uiText } from "../../i18n/uiText";
import { useRuntimeContext } from "../../runtime/context";
import type { AppConfig } from "../../runtime/types";

const {
  audioInputDevices,
  audioProbeError,
  audioProbeResult,
  config,
  currentSession,
  deleteProviderSecret,
  desiredRuntimePlan,
  isSecretsBusy,
  isSettingsBusy,
  isAudioProbeRunning,
  loadAudioInputDevices,
  pendingSessionChanges,
  probeAudioInput,
  saveConfig,
  saveProviderSecret,
  secretStatuses,
  secretsError,
  sessionUploadsMicrophoneAudio,
  settingsError,
} = useRuntimeContext();

const MICROPHONE_PROBE_DURATION_MS = 2_000;

const toast = useToast();
const activeSessionUploadsMicrophoneAudio = computed(
  () =>
    sessionUploadsMicrophoneAudio.value &&
    (currentSession.value?.phase === "starting" ||
      currentSession.value?.phase === "running" ||
      currentSession.value?.phase === "reconnecting"),
);

function handleTestMicrophone(inputDeviceId: string | null) {
  void probeAudioInput({
    inputDeviceId,
    durationMs: MICROPHONE_PROBE_DURATION_MS,
  });
}

async function handleSaveConfig(nextConfig: AppConfig, onSettled: () => void) {
  const didSave = await saveConfig(nextConfig).finally(onSettled);

  if (didSave) {
    toast.add({
      color: "success",
      icon: "i-lucide-circle-check",
      title: uiText("settings.feedback.saved"),
    });
  }
}
</script>

<template>
  <div class="grid gap-5">
    <header>
      <p class="text-xs font-semibold tracking-wide text-muted uppercase">
        {{ uiText("settings.title") }}
      </p>
      <h1 class="mt-1 text-2xl font-semibold tracking-tight text-highlighted">
        {{ uiText("settings.page.title") }}
      </h1>
    </header>

    <SettingsForm
      :audio-input-devices="audioInputDevices"
      :audio-probe-error="audioProbeError"
      :audio-probe-result="audioProbeResult"
      :config="config"
      :desired-runtime-plan="desiredRuntimePlan"
      :is-secrets-busy="isSecretsBusy"
      :is-settings-busy="isSettingsBusy"
      :is-audio-probe-running="isAudioProbeRunning"
      :pending-session-changes="pendingSessionChanges"
      :session-phase="currentSession?.phase ?? null"
      :active-session-uploads-microphone-audio="
        activeSessionUploadsMicrophoneAudio
      "
      :secret-statuses="secretStatuses"
      :secrets-error="secretsError"
      :settings-error="settingsError"
      @delete-provider-secret="deleteProviderSecret"
      @refresh-devices="loadAudioInputDevices"
      @save-config="handleSaveConfig"
      @save-provider-secret="saveProviderSecret"
      @test-microphone="handleTestMicrophone"
    />
  </div>
</template>

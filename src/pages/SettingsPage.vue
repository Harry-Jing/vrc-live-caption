<script setup lang="ts">
import { useToast } from "@nuxt/ui/composables";
import SettingsForm from "../components/SettingsForm.vue";
import { uiText } from "../i18n/uiText";
import { useRuntimeContext } from "../runtime/context";
import type { AppConfig } from "../runtime/types";

const {
  audioInputDevices,
  config,
  deleteProviderSecret,
  isSecretsBusy,
  isSettingsBusy,
  loadAudioInputDevices,
  requiresRuntimeRestart,
  saveConfig,
  saveProviderSecret,
  secretStatuses,
  secretsError,
  settingsError,
} = useRuntimeContext();

const toast = useToast();

async function handleSaveConfig(nextConfig: AppConfig) {
  const didSave = await saveConfig(nextConfig);

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
      :config="config"
      :is-secrets-busy="isSecretsBusy"
      :is-settings-busy="isSettingsBusy"
      :requires-runtime-restart="requiresRuntimeRestart"
      :secret-statuses="secretStatuses"
      :secrets-error="secretsError"
      :settings-error="settingsError"
      @delete-provider-secret="deleteProviderSecret"
      @refresh-devices="loadAudioInputDevices"
      @save-config="handleSaveConfig"
      @save-provider-secret="saveProviderSecret"
    />
  </div>
</template>

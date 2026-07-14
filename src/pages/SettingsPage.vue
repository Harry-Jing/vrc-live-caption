<script setup lang="ts">
import { useToast } from "@nuxt/ui/composables";
import SettingsForm from "../components/SettingsForm.vue";
import { useRuntimeContext } from "../runtime/context";
import type { AppConfig } from "../runtime/types";

const {
  audioInputDevices,
  config,
  deleteProviderSecret,
  isSecretsBusy,
  isSettingsBusy,
  loadAudioInputDevices,
  saveConfig,
  saveProviderSecret,
  secretStatuses,
  secretsError,
  settingsError,
  settingsNotice,
} = useRuntimeContext();

const toast = useToast();

async function handleSaveConfig(nextConfig: AppConfig) {
  const didSave = await saveConfig(nextConfig);

  if (didSave) {
    toast.add({
      color: "success",
      icon: "i-lucide-circle-check",
      title: "Settings saved",
    });
  }
}
</script>

<template>
  <div class="grid gap-5">
    <header>
      <p class="text-xs font-semibold tracking-wide text-muted uppercase">
        Settings
      </p>
      <h2 class="mt-1 text-2xl font-semibold tracking-tight text-highlighted">
        Capture, provider, and output
      </h2>
    </header>

    <SettingsForm
      :audio-input-devices="audioInputDevices"
      :config="config"
      :is-secrets-busy="isSecretsBusy"
      :is-settings-busy="isSettingsBusy"
      :secret-statuses="secretStatuses"
      :secrets-error="secretsError"
      :settings-error="settingsError"
      :settings-notice="settingsNotice"
      @delete-provider-secret="deleteProviderSecret"
      @refresh-devices="loadAudioInputDevices"
      @save-config="handleSaveConfig"
      @save-provider-secret="saveProviderSecret"
    />
  </div>
</template>

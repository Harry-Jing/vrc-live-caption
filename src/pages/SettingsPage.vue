<script setup lang="ts">
import SettingsForm from "../components/SettingsForm.vue";
import { useRuntimeContext } from "../runtime/context";

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
      @save-config="saveConfig"
      @save-provider-secret="saveProviderSecret"
    />
  </div>
</template>

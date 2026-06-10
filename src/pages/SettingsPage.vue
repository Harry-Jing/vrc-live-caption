<script setup lang="ts">
import ControlPanel from "../components/ControlPanel.vue";
import { useRuntimeContext } from "../runtime/context";

const {
  actionError,
  audioInputDevices,
  config,
  deleteProviderSecret,
  isBusy,
  loadAudioInputDevices,
  openAiSecretStatus,
  saveConfig,
  saveProviderSecret,
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

    <ControlPanel
      :action-error="actionError"
      :audio-input-devices="audioInputDevices"
      :config="config"
      :is-busy="isBusy"
      :open-ai-secret-status="openAiSecretStatus"
      @delete-provider-secret="deleteProviderSecret"
      @refresh-devices="loadAudioInputDevices"
      @save-config="saveConfig"
      @save-provider-secret="saveProviderSecret"
    />
  </div>
</template>

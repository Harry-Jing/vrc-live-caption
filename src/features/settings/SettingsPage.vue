<script setup lang="ts">
import { useToast } from "@nuxt/ui/composables";
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { onBeforeRouteLeave } from "vue-router";
import MicrophoneProbeControl from "./MicrophoneProbeControl.vue";
import { uiText } from "../../i18n/uiText";
import { requestConfirmation } from "../../platform/confirmation";
import { useRuntimeContext } from "../../runtime/context";
import { isActiveRuntimeGenerationPhase } from "../../runtime/lifecycle";
import {
  publicationModeDescriptionMessageKey,
  publicationModeMessageKey,
  publicationPlanDescription,
  publicationSettingsView,
  recognitionPathDescriptionMessageKey,
  recognitionPathMessageKey,
} from "../../runtime/presentation";
import {
  PUBLICATION_MODES,
  RECOGNITION_PATHS,
  type PublicationMode,
} from "../../runtime/captionPipeline";
import { openAiCredentialStatusPresentation } from "./openAiCredentialStatusPresentation";
import { useSettingsDraft } from "./settingsDraft";

const {
  audioInputDevices,
  audioProbeFailure,
  audioProbeResult,
  credentialFailure,
  credentialStatuses,
  currentGeneration,
  currentGenerationUploadsMicrophoneAudio,
  deleteCredential,
  desiredCaptionPipelinePlan,
  desiredConfig,
  isAudioProbeRunning,
  isCredentialBusy,
  isSettingsBusy,
  loadAudioInputDevices,
  pendingGenerationChanges,
  probeAudioInput,
  saveConfig,
  saveCredential,
  settingsFailure,
} = useRuntimeContext();

const MICROPHONE_PROBE_DURATION_MS = 2_000;
const toast = useToast();
const activeGenerationUploadsMicrophoneAudio = computed(
  () =>
    currentGenerationUploadsMicrophoneAudio.value &&
    (currentGeneration.value?.phase === "starting" ||
      currentGeneration.value?.phase === "running" ||
      currentGeneration.value?.phase === "reconnecting"),
);

const {
  createSaveConfig,
  draft: form,
  hasValidExpectedLanguages,
  isDirty: isFormDirty,
} = useSettingsDraft(() => desiredConfig.value);
const apiKeyInput = ref("");
const isConfigSaveSubmitting = ref(false);
const isRemoveKeyModalOpen = ref(false);
const recognitionFields = ref<HTMLElement | null>(null);
const lastTestedInputDeviceId = ref<string | null | undefined>(undefined);

const openAiCredentialStatus = computed(
  () => credentialStatuses.value.openai ?? null,
);
const openAiCredentialPresentation = computed(() =>
  openAiCredentialStatusPresentation(openAiCredentialStatus.value),
);
const areConfigControlsDisabled = computed(
  () => isConfigSaveSubmitting.value || isSettingsBusy.value,
);
const isRuntimeActive = computed(() =>
  isActiveRuntimeGenerationPhase(currentGeneration.value?.phase),
);

const probeMatchesSelectedInput = computed(
  () =>
    lastTestedInputDeviceId.value !== undefined &&
    lastTestedInputDeviceId.value === form.value?.audio.inputDeviceId,
);

const visibleAudioProbeResult = computed(() =>
  probeMatchesSelectedInput.value ? audioProbeResult.value : null,
);

const visibleAudioProbeError = computed(() =>
  probeMatchesSelectedInput.value
    ? (audioProbeFailure.value?.message ?? "")
    : "",
);

const settingsFailureMessage = computed(
  () => settingsFailure.value?.message ?? "",
);
const credentialFailureMessage = computed(
  () => credentialFailure.value?.message ?? "",
);

const canSaveOpenAiApiKey = computed(() => apiKeyInput.value.trim().length > 0);

const pendingGenerationChangesDescription = computed(() => {
  const changes = pendingGenerationChanges.value
    .map((change) => {
      switch (change) {
        case "microphone":
          return uiText("settings.feedback.nextStart.change.microphone");
        case "recognition":
          return uiText("settings.feedback.nextStart.change.recognition");
        case "credential":
          return uiText("settings.feedback.nextStart.change.credential");
        case "chatboxOutput":
          return uiText("settings.feedback.nextStart.change.chatboxOutput");
        case "publication":
          return uiText("settings.feedback.nextStart.change.publication");
      }
    })
    .join(", ");

  return uiText(
    currentGeneration.value?.phase === "error"
      ? "settings.feedback.nextStart.failedDescription"
      : "settings.feedback.nextStart.description",
    { changes },
  );
});

const removeOpenAiCredentialDescription = computed(() =>
  uiText(
    activeGenerationUploadsMicrophoneAudio.value
      ? "settings.credentials.openai.removeDialog.currentGenerationDescription"
      : "settings.credentials.openai.removeDialog.description",
  ),
);

// Sentinel for "use the system default device": the config stores null, but
// reka-ui's Select forbids empty-string item values.
const DEFAULT_DEVICE_VALUE = "__default-input-device__";

const inputDeviceItems = computed(() => {
  const items = [
    {
      label: uiText("audio.devices.defaultInput"),
      value: DEFAULT_DEVICE_VALUE,
    },
    ...audioInputDevices.value.map((device) => ({
      label: device.isDefault
        ? uiText("audio.devices.defaultNamed", { name: device.name })
        : device.name,
      value: device.id,
    })),
  ];
  const selectedId = form.value?.audio.inputDeviceId;

  // Keep a saved-but-disconnected device selectable instead of showing a
  // blank select; the user can keep waiting for it or pick another device.
  if (
    selectedId &&
    !audioInputDevices.value.some((device) => device.id === selectedId)
  ) {
    items.push({
      label: uiText("audio.devices.savedDisconnected"),
      value: selectedId,
    });
  }

  return items;
});

const recognitionPathItems = RECOGNITION_PATHS.map((value) => ({
  label: uiText(recognitionPathMessageKey[value]),
  value,
}));

const selectedRecognitionPathDescription = computed(() => {
  const path = form.value?.recognition.path;

  return path ? uiText(recognitionPathDescriptionMessageKey[path]) : "";
});

const publicationModeItems = PUBLICATION_MODES.map((value) => ({
  label: uiText(publicationModeMessageKey[value]),
  description: uiText(publicationModeDescriptionMessageKey[value]),
  value,
}));

const publicationView = computed(() =>
  publicationSettingsView(desiredCaptionPipelinePlan.value, isFormDirty.value),
);

const publicationDescription = computed(() =>
  publicationView.value.state === "compatible"
    ? publicationPlanDescription(publicationView.value)
    : "",
);

async function confirmDiscardDraft() {
  if (!isFormDirty.value) {
    return true;
  }

  return requestConfirmation(uiText("settings.unsavedChanges.confirmLeave"));
}

function handleBeforeUnload(event: BeforeUnloadEvent) {
  if (!isFormDirty.value) {
    return;
  }

  event.preventDefault();
}

onBeforeRouteLeave(() => confirmDiscardDraft());
onMounted(() => {
  window.addEventListener("beforeunload", handleBeforeUnload);
});
onBeforeUnmount(() => {
  window.removeEventListener("beforeunload", handleBeforeUnload);
});

const selectedInputDevice = computed({
  get: () => form.value?.audio.inputDeviceId ?? DEFAULT_DEVICE_VALUE,
  set: (value: string) => {
    if (form.value) {
      form.value.audio.inputDeviceId =
        value === DEFAULT_DEVICE_VALUE ? null : value;
    }
  },
});

async function save() {
  if (areConfigControlsDisabled.value) {
    return;
  }

  const next = createSaveConfig();
  if (next === null) {
    return;
  }

  isConfigSaveSubmitting.value = true;
  try {
    const didSave = await saveConfig(next);
    if (didSave) {
      toast.add({
        color: "success",
        icon: "i-lucide-circle-check",
        title: uiText("settings.feedback.saved"),
      });
    }
  } finally {
    isConfigSaveSubmitting.value = false;
  }
}

function saveOpenAiApiKey() {
  void saveCredential("openai", apiKeyInput.value);
  // Do not retain plaintext in the form while waiting for secure-store I/O.
  // Full control snapshots are unrelated acknowledgements and must never be
  // used to decide when this local secret input is cleared.
  apiKeyInput.value = "";
}

function testMicrophone() {
  const inputDeviceId = form.value?.audio.inputDeviceId;
  if (inputDeviceId === undefined) {
    return;
  }

  lastTestedInputDeviceId.value = inputDeviceId;
  void probeAudioInput({
    inputDeviceId,
    durationMs: MICROPHONE_PROBE_DURATION_MS,
  });
}

function requestDeleteOpenAiApiKey() {
  isRemoveKeyModalOpen.value = true;
}

function closeRemoveKeyModal() {
  isRemoveKeyModalOpen.value = false;
}

function confirmDeleteOpenAiApiKey() {
  isRemoveKeyModalOpen.value = false;
  void deleteCredential("openai");
}

function selectPublicationMode(mode: PublicationMode) {
  if (form.value) {
    form.value.publication.mode = mode;
  }
}

async function focusRecognitionPath() {
  await nextTick();

  recognitionFields.value?.scrollIntoView({
    behavior: "auto",
    block: "center",
  });
  recognitionFields.value?.querySelector<HTMLElement>("button, input")?.focus();
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

    <UCard :ui="{ body: 'p-5' }">
      <template #header>
        <div class="flex items-start justify-between gap-4">
          <div>
            <h2 class="text-base font-semibold text-highlighted">
              {{ uiText("settings.title") }}
            </h2>
            <p class="mt-1 text-sm text-muted">
              {{ uiText("settings.description") }}
            </p>
          </div>
          <UButton
            :disabled="areConfigControlsDisabled"
            icon="i-lucide-refresh-cw"
            :label="uiText('settings.actions.refreshDevices')"
            size="sm"
            variant="ghost"
            @click="loadAudioInputDevices"
          />
        </div>
      </template>

      <UAlert
        v-if="settingsFailureMessage"
        class="mb-4"
        color="error"
        icon="i-lucide-circle-alert"
        role="alert"
        :title="uiText('settings.errors.actionFailed')"
        :description="settingsFailureMessage"
        variant="subtle"
      />

      <UAlert
        v-if="pendingGenerationChanges.length > 0"
        class="mb-4"
        color="warning"
        icon="i-lucide-triangle-alert"
        :title="uiText('settings.feedback.nextStart.title')"
        :description="pendingGenerationChangesDescription"
        variant="subtle"
      />

      <form v-if="form" class="grid gap-5" @submit.prevent="save">
        <section class="grid gap-4">
          <h3 class="text-sm font-semibold text-highlighted">
            {{ uiText("settings.sections.audio") }}
          </h3>

          <UFormField :label="uiText('settings.fields.microphone')">
            <USelect
              v-model="selectedInputDevice"
              class="w-full"
              :disabled="areConfigControlsDisabled"
              :items="inputDeviceItems"
            />
          </UFormField>

          <MicrophoneProbeControl
            :disabled="areConfigControlsDisabled"
            :error="visibleAudioProbeError"
            :is-running="isAudioProbeRunning"
            :result="visibleAudioProbeResult"
            :runtime-active="isRuntimeActive"
            @test="testMicrophone"
          />
        </section>

        <USeparator />

        <section ref="recognitionFields" class="grid gap-4">
          <h3 class="text-sm font-semibold text-highlighted">
            {{ uiText("settings.sections.recognition") }}
          </h3>

          <div class="grid gap-3 sm:grid-cols-2">
            <UFormField
              :label="uiText('settings.fields.recognitionPath')"
              :description="selectedRecognitionPathDescription"
            >
              <USelect
                v-model="form.recognition.path"
                class="w-full"
                :disabled="areConfigControlsDisabled"
                :items="recognitionPathItems"
              />
            </UFormField>

            <UFormField
              :label="uiText('settings.fields.language')"
              :description="uiText('settings.fields.language.description')"
              :error="
                hasValidExpectedLanguages
                  ? false
                  : uiText('settings.fields.language.required')
              "
            >
              <UInputTags
                v-model="form.recognition.expectedLanguages"
                add-on-blur
                class="w-full"
                :disabled="areConfigControlsDisabled"
              />
            </UFormField>
          </div>

          <div
            class="grid gap-3 rounded-md border border-default bg-muted/30 p-3"
          >
            <div class="flex items-center justify-between gap-3">
              <span class="text-sm font-medium text-highlighted">
                {{ uiText("settings.credentials.openai.title") }}
              </span>
              <UBadge
                :color="openAiCredentialPresentation.color"
                variant="subtle"
              >
                {{ openAiCredentialPresentation.label }}
              </UBadge>
            </div>

            <p class="text-sm text-muted">
              {{ uiText("settings.credentials.openai.cloudDisclosure") }}
            </p>

            <div
              class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-end"
            >
              <UFormField :label="uiText('settings.credentials.openai.apiKey')">
                <UInput
                  v-model="apiKeyInput"
                  autocapitalize="off"
                  autocomplete="off"
                  class="w-full"
                  :placeholder="
                    uiText('settings.credentials.openai.apiKeyPlaceholder')
                  "
                  spellcheck="false"
                  type="password"
                />
              </UFormField>
              <UButton
                :disabled="isCredentialBusy || !canSaveOpenAiApiKey"
                icon="i-lucide-key-round"
                :label="uiText('settings.credentials.openai.actions.save')"
                type="button"
                variant="subtle"
                @click="saveOpenAiApiKey"
              />
              <UButton
                v-if="openAiCredentialPresentation.canRemove"
                :disabled="isCredentialBusy"
                color="error"
                icon="i-lucide-trash-2"
                :label="uiText('settings.credentials.openai.actions.remove')"
                type="button"
                variant="ghost"
                @click="requestDeleteOpenAiApiKey"
              />
            </div>

            <UAlert
              v-if="credentialFailureMessage"
              color="error"
              icon="i-lucide-circle-alert"
              role="alert"
              :title="uiText('settings.credentials.openai.errors.actionFailed')"
              :description="credentialFailureMessage"
              variant="subtle"
            />

            <p
              v-if="openAiCredentialPresentation.failureMessage"
              class="text-xs text-error"
              role="alert"
            >
              {{ openAiCredentialPresentation.failureMessage }}
            </p>
          </div>
        </section>

        <USeparator />

        <section class="grid gap-4">
          <h3 class="text-sm font-semibold text-highlighted">
            {{ uiText("settings.sections.chatboxOutput") }}
          </h3>

          <UFormField
            :label="uiText('settings.fields.publicationMode')"
            :description="uiText('settings.publication.description')"
          >
            <URadioGroup
              v-model="form.publication.mode"
              :disabled="areConfigControlsDisabled"
              :items="publicationModeItems"
              :legend="uiText('settings.fields.publicationMode')"
              name="publicationMode"
              orientation="horizontal"
              :ui="{ legend: 'sr-only' }"
              variant="card"
            />
          </UFormField>

          <p
            v-if="publicationView.state === 'unavailable'"
            class="text-xs text-muted"
          >
            {{ uiText("settings.publication.loading") }}
          </p>

          <UAlert
            v-else-if="publicationView.state === 'unverified'"
            color="info"
            icon="i-lucide-info"
            :title="uiText('settings.publication.unverified.title')"
            :description="uiText('settings.publication.unverified.description')"
            variant="subtle"
          />

          <UAlert
            v-else-if="publicationView.state === 'incompatible'"
            color="error"
            icon="i-lucide-circle-alert"
            role="alert"
            :title="
              uiText('settings.publication.incompatible.title', {
                mode: uiText(publicationModeMessageKey[publicationView.mode]),
              })
            "
            :description="
              uiText('settings.publication.incompatible.description')
            "
            variant="subtle"
          >
            <template #actions>
              <UButton
                v-for="mode in publicationView.supportedModes"
                :key="mode"
                color="neutral"
                :label="
                  uiText('settings.publication.incompatible.useMode', {
                    mode: uiText(publicationModeMessageKey[mode]),
                  })
                "
                size="xs"
                type="button"
                variant="outline"
                @click="selectPublicationMode(mode)"
              />
              <UButton
                color="neutral"
                :label="
                  uiText('settings.publication.incompatible.changePath', {
                    mode: uiText(
                      publicationModeMessageKey[publicationView.mode],
                    ),
                  })
                "
                size="xs"
                type="button"
                variant="outline"
                @click="focusRecognitionPath"
              />
            </template>
          </UAlert>

          <p v-else class="text-xs text-muted">
            {{
              uiText("settings.publication.ready", {
                description: publicationDescription,
              })
            }}
          </p>

          <div class="grid gap-3 sm:grid-cols-[1fr_140px]">
            <UFormField :label="uiText('settings.fields.oscHost')">
              <UInput
                v-model="form.osc.host"
                class="w-full"
                :disabled="areConfigControlsDisabled"
              />
            </UFormField>

            <UFormField :label="uiText('settings.fields.port')">
              <UInputNumber
                v-model="form.osc.port"
                class="w-full"
                :disabled="areConfigControlsDisabled"
                :format-options="{ useGrouping: false }"
                :max="65535"
                :min="1"
              />
            </UFormField>
          </div>

          <div class="grid gap-3 sm:grid-cols-2">
            <USwitch
              v-model="form.osc.enabled"
              :disabled="areConfigControlsDisabled"
              :label="uiText('settings.fields.chatboxOutput')"
            />
            <USwitch
              v-model="form.ui.showOngoingPreview"
              :disabled="areConfigControlsDisabled"
              :label="uiText('settings.fields.ongoingPreview')"
            />
          </div>
        </section>

        <UButton
          :disabled="areConfigControlsDisabled || !hasValidExpectedLanguages"
          icon="i-lucide-save"
          :label="uiText('settings.actions.save')"
          type="submit"
          block
        />
      </form>

      <p v-else class="text-sm text-muted">
        {{
          settingsFailureMessage
            ? uiText("settings.loadFailed")
            : uiText("settings.loading")
        }}
      </p>
    </UCard>

    <UModal
      v-model:open="isRemoveKeyModalOpen"
      :title="uiText('settings.credentials.openai.removeDialog.title')"
      :description="removeOpenAiCredentialDescription"
    >
      <template #footer>
        <div class="flex w-full justify-end gap-2">
          <UButton
            color="neutral"
            :label="uiText('settings.credentials.openai.removeDialog.cancel')"
            variant="outline"
            @click="closeRemoveKeyModal"
          />
          <UButton
            :disabled="isCredentialBusy"
            color="error"
            icon="i-lucide-trash-2"
            :label="uiText('settings.credentials.openai.removeDialog.confirm')"
            @click="confirmDeleteOpenAiApiKey"
          />
        </div>
      </template>
    </UModal>
  </div>
</template>

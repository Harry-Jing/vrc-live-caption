<script setup lang="ts">
import { useToast } from "@nuxt/ui/composables";
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { onBeforeRouteLeave } from "vue-router";
import CompletedTranslationSettings from "./CompletedTranslationSettings.vue";
import MicrophoneProbeControl from "./MicrophoneProbeControl.vue";
import ServiceCredentialControl from "./ServiceCredentialControl.vue";
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
import type { CredentialId } from "../../runtime/runtimeControl";
import type { ServiceCredentialControlCopy } from "./serviceCredentialControl";
import { useSettingsDraft } from "./settingsDraft";

const {
  audioInputDevices,
  audioProbeFailure,
  audioProbeResult,
  credentialOperationStates,
  credentialStatuses,
  currentGeneration,
  deleteCredential,
  desiredCaptionPipelinePlan,
  desiredConfig,
  isAudioProbeRunning,
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

const {
  canSave,
  createSaveConfig,
  draft: form,
  hasValidExpectedLanguages,
  isDirty: isFormDirty,
  selectContent,
  selectTranslationEndpoint,
  selectTranslationTarget,
  setCustomTranslationApiBaseUrl,
  translationIssues,
} = useSettingsDraft(() => desiredConfig.value);
const isConfigSaveSubmitting = ref(false);
const recognitionFields = ref<HTMLElement | null>(null);
const lastTestedInputDeviceId = ref<string | null | undefined>(undefined);

const openAiCredentialStatus = computed(
  () => credentialStatuses.value.openai ?? null,
);
const customTranslationCredentialStatus = computed(
  () => credentialStatuses.value.customTranslation ?? null,
);
const openAiCredentialOperation = computed(
  () => credentialOperationStates.value.openai,
);
const customTranslationCredentialOperation = computed(
  () => credentialOperationStates.value.customTranslation,
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

const pendingGenerationChangesDescription = computed(() => {
  const changes = pendingGenerationChanges.value
    .map((change) => {
      switch (change) {
        case "microphone":
          return uiText("settings.feedback.nextStart.change.microphone");
        case "recognition":
          return uiText("settings.feedback.nextStart.change.recognition");
        case "translation":
          return uiText("settings.feedback.nextStart.change.translation");
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

const activeGenerationCredentialIds = computed<ReadonlySet<CredentialId>>(
  () => {
    const phase = currentGeneration.value?.phase;
    if (
      phase !== "starting" &&
      phase !== "running" &&
      phase !== "reconnecting"
    ) {
      return new Set<CredentialId>();
    }

    return new Set(
      currentGeneration.value?.credentials.map((credential) => credential.id),
    );
  },
);

const shouldShowCustomTranslationCredential = computed(
  () =>
    form.value?.publication.content !== "sourceOnly" &&
    form.value?.translation?.endpointKind === "custom",
);

const openAiCredentialCopy: ServiceCredentialControlCopy = {
  title: uiText("settings.credentials.openai.title"),
  disclosure: uiText("settings.credentials.openai.cloudDisclosure"),
  inputLabel: uiText("settings.credentials.openai.apiKey"),
  inputPlaceholder: uiText("settings.credentials.openai.apiKeyPlaceholder"),
  save: uiText("settings.credentials.openai.actions.save"),
  replace: uiText("settings.credentials.openai.actions.replace"),
  remove: uiText("settings.credentials.openai.actions.remove"),
  actionFailed: uiText("settings.credentials.openai.errors.actionFailed"),
  removeDialogTitle: uiText("settings.credentials.openai.removeDialog.title"),
  removeDialogDescription: uiText(
    "settings.credentials.openai.removeDialog.description",
  ),
  removeDialogCurrentGenerationDescription: uiText(
    "settings.credentials.openai.removeDialog.currentGenerationDescription",
  ),
  removeDialogCancel: uiText("settings.credentials.openai.removeDialog.cancel"),
  removeDialogConfirm: uiText(
    "settings.credentials.openai.removeDialog.confirm",
  ),
};

const customTranslationCredentialCopy: ServiceCredentialControlCopy = {
  title: uiText("settings.credentials.customTranslation.title"),
  disclosure: uiText("settings.credentials.customTranslation.disclosure"),
  inputLabel: uiText("settings.credentials.customTranslation.apiKey"),
  inputPlaceholder: uiText(
    "settings.credentials.customTranslation.apiKeyPlaceholder",
  ),
  save: uiText("settings.credentials.customTranslation.actions.save"),
  replace: uiText("settings.credentials.customTranslation.actions.replace"),
  remove: uiText("settings.credentials.customTranslation.actions.remove"),
  actionFailed: uiText(
    "settings.credentials.customTranslation.errors.actionFailed",
  ),
  removeDialogTitle: uiText(
    "settings.credentials.customTranslation.removeDialog.title",
  ),
  removeDialogDescription: uiText(
    "settings.credentials.customTranslation.removeDialog.description",
  ),
  removeDialogCurrentGenerationDescription: uiText(
    "settings.credentials.customTranslation.removeDialog.currentGenerationDescription",
  ),
  removeDialogCancel: uiText(
    "settings.credentials.customTranslation.removeDialog.cancel",
  ),
  removeDialogConfirm: uiText(
    "settings.credentials.customTranslation.removeDialog.confirm",
  ),
};

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

function saveOpenAiApiKey(secret: string) {
  void saveCredential("openai", secret);
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

function deleteOpenAiApiKey() {
  void deleteCredential("openai");
}

function saveCustomTranslationApiKey(secret: string) {
  void saveCredential("customTranslation", secret);
}

function deleteCustomTranslationApiKey() {
  void deleteCredential("customTranslation");
}

function useCompletedPublication() {
  if (form.value) {
    form.value.publication.mode = "completed";
  }
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
              :ui="{
                fieldset: 'flex-wrap',
                item: 'min-w-56 flex-1',
                legend: 'sr-only',
              }"
              variant="card"
            />
          </UFormField>

          <USeparator />

          <CompletedTranslationSettings
            :content="form.publication.content"
            :custom-credential-status="customTranslationCredentialStatus"
            :disabled="areConfigControlsDisabled"
            :issues="translationIssues"
            :open-ai-credential-status="openAiCredentialStatus"
            :publication-mode="form.publication.mode"
            :translation="form.translation"
            @select-content="selectContent"
            @select-endpoint="selectTranslationEndpoint"
            @select-target="selectTranslationTarget"
            @set-custom-api-base-url="setCustomTranslationApiBaseUrl"
            @use-completed="useCompletedPublication"
          />

          <USeparator />

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
                v-if="form.publication.content === 'sourceOnly'"
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
          :disabled="areConfigControlsDisabled || !canSave"
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

    <UCard :ui="{ body: 'p-5' }">
      <template #header>
        <div>
          <h2 class="text-base font-semibold text-highlighted">
            {{ uiText("settings.sections.serviceCredentials") }}
          </h2>
          <p class="mt-1 text-sm text-muted">
            {{ uiText("settings.credentials.description") }}
          </p>
        </div>
      </template>

      <div class="grid gap-4">
        <ServiceCredentialControl
          :action-failure="openAiCredentialOperation.failure?.message ?? ''"
          :busy="openAiCredentialOperation.isBusy"
          :captured-by-active-generation="
            activeGenerationCredentialIds.has('openai')
          "
          :copy="openAiCredentialCopy"
          :status="openAiCredentialStatus"
          @delete="deleteOpenAiApiKey"
          @save="saveOpenAiApiKey"
        />

        <ServiceCredentialControl
          v-if="shouldShowCustomTranslationCredential"
          :action-failure="
            customTranslationCredentialOperation.failure?.message ?? ''
          "
          :busy="customTranslationCredentialOperation.isBusy"
          :captured-by-active-generation="
            activeGenerationCredentialIds.has('customTranslation')
          "
          :copy="customTranslationCredentialCopy"
          :status="customTranslationCredentialStatus"
          @delete="deleteCustomTranslationApiKey"
          @save="saveCustomTranslationApiKey"
        />
      </div>
    </UCard>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import CaptionPreview from "./CaptionPreview.vue";
import MicrophoneLevelMeter from "./MicrophoneLevelMeter.vue";
import RuntimeControls from "./RuntimeControls.vue";
import { uiText } from "../../i18n/uiText";
import { formatTime } from "../../i18n/format";
import { isActiveRuntimeGenerationPhase } from "../../runtime/lifecycle";
import { useRuntimeContext } from "../../runtime/context";
import {
  publicationDisplayPlanView,
  publicationModeMessageKey,
  publicationPlanDescription,
  publicationStartIsBlocked,
  recognitionPathMessageKey,
  recognitionPathServiceMessageKey,
} from "../../runtime/presentation";

const {
  audioInputDevices,
  captionPreviewStatus,
  completedCaptions,
  currentGeneration,
  currentGenerationCaptionPipelinePlan,
  currentGenerationSelection,
  desiredCaptionPipelinePlan,
  desiredConfig,
  diagnostics,
  inFlightRuntimeAction,
  isRuntimeBusy,
  latestAudioLevel,
  pendingGenerationChanges,
  runAction,
  runtimeFailure,
  runtimeStatus,
  visibleCaptionText,
} = useRuntimeContext();

const runtimeFailureMessage = computed(
  () => runtimeFailure.value?.message ?? "",
);

const latestDiagnostic = computed(() => diagnostics.value.at(0) ?? null);
const latestCompletedCaption = computed(
  () => completedCaptions.value.at(0) ?? null,
);

const currentMicrophoneLabel = computed(() => {
  const selection = currentGenerationSelection.value;
  const config = desiredConfig.value;

  if (!selection && !config) {
    return uiText("common.loading");
  }

  const selectedId = selection
    ? selection.audio.inputDeviceId
    : config?.audio.inputDeviceId;

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

const currentOscTarget = computed(() => {
  const generation = currentGeneration.value;

  if (generation) {
    return `${generation.chatboxPublication.host}:${String(generation.chatboxPublication.port)}`;
  }

  const config = desiredConfig.value;

  return config
    ? `${config.osc.host}:${String(config.osc.port)}`
    : uiText("common.loading");
});

const chatboxBadge = computed(() => {
  const generation = currentGeneration.value;

  if (generation?.chatboxPublication.state === "unavailable") {
    return {
      color: "error" as const,
      label: uiText("captioning.chatbox.unavailable"),
    };
  }

  const enabled = generation
    ? generation.chatboxPublication.state === "ready"
    : (desiredConfig.value?.osc.enabled ?? false);

  return {
    color: enabled ? ("success" as const) : ("neutral" as const),
    label: uiText(enabled ? "captioning.chatbox.on" : "captioning.chatbox.off"),
  };
});

const pendingGenerationChangesDescription = computed(() =>
  uiText(
    currentGeneration.value?.phase === "error"
      ? "captioning.currentSetup.pendingChanges.failedDescription"
      : "captioning.currentSetup.pendingChanges.description",
  ),
);

const hasActiveGeneration = computed(() =>
  isActiveRuntimeGenerationPhase(currentGeneration.value?.phase),
);

const currentPublication = computed(() =>
  publicationDisplayPlanView(
    currentGenerationCaptionPipelinePlan.value,
    desiredCaptionPipelinePlan.value,
  ),
);

const currentPublicationLabel = computed(() => {
  const publication = currentPublication.value;

  if (publication.state === "unavailable") {
    return uiText("captioning.publication.unavailable");
  }

  const mode = uiText(publicationModeMessageKey[publication.mode]);

  if (publication.state === "incompatible") {
    return uiText("captioning.publication.incompatibleValue", { mode });
  }

  return uiText("captioning.publication.readyValue", {
    description: publicationPlanDescription(publication),
    mode,
  });
});

const isStartBlocked = computed(() =>
  publicationStartIsBlocked(
    hasActiveGeneration.value,
    desiredCaptionPipelinePlan.value,
  ),
);

const currentRecognitionPath = computed(() =>
  currentGenerationSelection.value
    ? currentGenerationSelection.value.recognition.path
    : (desiredConfig.value?.recognition.path ?? null),
);
</script>

<template>
  <div class="grid gap-5">
    <header
      class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between"
    >
      <div>
        <p class="text-xs font-semibold tracking-wide text-muted uppercase">
          {{ uiText("captioning.eyebrow") }}
        </p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight text-highlighted">
          {{ uiText("captioning.title") }}
        </h1>
      </div>

      <div class="flex flex-wrap gap-2">
        <UBadge
          v-if="desiredConfig && !currentGeneration"
          color="info"
          variant="subtle"
        >
          {{ uiText("captioning.currentSetup.nextStartBadge") }}
        </UBadge>
        <UBadge color="neutral" variant="subtle">
          {{
            currentRecognitionPath
              ? uiText(recognitionPathServiceMessageKey[currentRecognitionPath])
              : uiText("common.loading")
          }}
        </UBadge>
        <UBadge :color="chatboxBadge.color" variant="subtle">
          {{ chatboxBadge.label }}
        </UBadge>
      </div>
    </header>

    <UAlert
      v-if="pendingGenerationChanges.length > 0"
      color="warning"
      icon="i-lucide-triangle-alert"
      :title="uiText('captioning.currentSetup.pendingChanges.title')"
      :description="pendingGenerationChangesDescription"
      variant="subtle"
    />

    <UAlert
      v-if="isStartBlocked"
      color="error"
      icon="i-lucide-circle-alert"
      role="alert"
      :title="uiText('captioning.publication.blocked.title')"
      :description="uiText('captioning.publication.blocked.description')"
      variant="subtle"
    >
      <template #actions>
        <UButton
          color="neutral"
          :label="uiText('captioning.publication.blocked.action')"
          size="xs"
          to="/settings"
          variant="outline"
        />
      </template>
    </UAlert>

    <CaptionPreview
      :latest-completed-caption="latestCompletedCaption"
      :status="captionPreviewStatus"
      :text="visibleCaptionText"
    />

    <div class="grid gap-5 xl:grid-cols-[minmax(0,1fr)_340px]">
      <RuntimeControls
        :error-message="runtimeFailureMessage"
        :in-flight-action="inFlightRuntimeAction"
        :is-busy="isRuntimeBusy"
        :is-start-blocked="isStartBlocked"
        :runtime-status="runtimeStatus"
        @run="runAction"
      />

      <div class="grid gap-5">
        <MicrophoneLevelMeter
          :generation="currentGeneration?.id ?? null"
          :level="latestAudioLevel"
          :generation-phase="currentGeneration?.phase ?? null"
        />

        <UCard :ui="{ body: 'p-5' }">
          <template #header>
            <div class="flex items-center justify-between gap-4">
              <h2 class="text-base font-semibold text-highlighted">
                {{
                  uiText(
                    currentGeneration?.phase === "error"
                      ? "captioning.currentSetup.failedGenerationTitle"
                      : currentGeneration
                        ? "captioning.currentSetup.activeGenerationTitle"
                        : desiredConfig
                          ? "captioning.currentSetup.nextStartTitle"
                          : "captioning.currentSetup.title",
                  )
                }}
              </h2>
              <UButton
                :label="uiText('captioning.currentSetup.edit')"
                size="sm"
                to="/settings"
                variant="link"
              />
            </div>
          </template>

          <dl class="grid gap-3 text-sm">
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("captioning.currentSetup.microphone") }}
              </dt>
              <dd
                class="min-w-0 text-right font-medium break-words text-highlighted"
              >
                {{ currentMicrophoneLabel }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("captioning.currentSetup.recognitionPath") }}
              </dt>
              <dd class="text-right font-medium text-highlighted">
                {{
                  currentRecognitionPath
                    ? uiText(recognitionPathMessageKey[currentRecognitionPath])
                    : uiText("common.loading")
                }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("captioning.currentSetup.publication") }}
              </dt>
              <dd
                class="min-w-0 text-right font-medium break-words text-highlighted"
              >
                {{ currentPublicationLabel }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("captioning.currentSetup.oscTarget") }}
              </dt>
              <dd class="font-medium text-highlighted">
                {{ currentOscTarget }}
              </dd>
            </div>
          </dl>
        </UCard>

        <UCard :ui="{ body: 'p-5' }">
          <template #header>
            <div class="flex items-center justify-between gap-4">
              <h2 class="text-base font-semibold text-highlighted">
                {{ uiText("captioning.recentActivity.title") }}
              </h2>
              <UButton
                :label="uiText('captioning.recentActivity.open')"
                size="sm"
                to="/diagnostics"
                variant="link"
              />
            </div>
          </template>

          <div class="grid gap-4 text-sm">
            <div>
              <p class="text-muted">
                {{ uiText("captioning.recentActivity.latestCompletedCaption") }}
              </p>
              <p class="mt-1 leading-6 break-words text-highlighted">
                {{
                  latestCompletedCaption?.text ??
                  uiText("captioning.recentActivity.noCompletedCaption")
                }}
              </p>
            </div>

            <USeparator />

            <div>
              <p class="text-muted">
                {{ uiText("captioning.recentActivity.latestDiagnostic") }}
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

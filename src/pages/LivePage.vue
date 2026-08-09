<script setup lang="ts">
import { computed } from "vue";
import CaptionPreview from "../components/CaptionPreview.vue";
import LiveAudioMeter from "../components/LiveAudioMeter.vue";
import RuntimeControls from "../components/RuntimeControls.vue";
import { uiText } from "../i18n/uiText";
import { formatTime } from "../runtime/format";
import { isActiveRuntimeSessionPhase } from "../runtime/lifecycle";
import { useRuntimeContext } from "../runtime/context";
import {
  publicationDisplayPlanView,
  publicationModeMessageKey,
  publicationPlanDescription,
  publicationStartIsBlocked,
  sttProviderMessageKey,
} from "../runtime/presentation";

const {
  audioInputDevices,
  captionPreviewStatus,
  completedCaptions,
  currentSession,
  currentSetupConfig,
  diagnostics,
  desiredRuntimePlan,
  inFlightRuntimeCommand,
  isRuntimeBusy,
  latestAudioLevel,
  pendingSessionChanges,
  runCommand,
  runtimeError,
  runtimeStatus,
  sessionRuntimePlan,
  visibleCaptionText,
} = useRuntimeContext();

const latestDiagnostic = computed(() => diagnostics.value.at(0) ?? null);
const latestCompletedCaption = computed(
  () => completedCaptions.value.at(0) ?? null,
);

const currentMicrophoneLabel = computed(() => {
  const currentConfig = currentSetupConfig.value;

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

const currentOscTarget = computed(() => {
  const session = currentSession.value;

  if (session) {
    return `${session.chatbox.host}:${String(session.chatbox.port)}`;
  }

  const currentConfig = currentSetupConfig.value;

  return currentConfig
    ? `${currentConfig.osc.host}:${String(currentConfig.osc.port)}`
    : uiText("common.loading");
});

const chatboxBadge = computed(() => {
  const session = currentSession.value;

  if (session?.chatbox.state === "unavailable") {
    return {
      color: "error" as const,
      label: uiText("live.chatbox.unavailable"),
    };
  }

  const enabled = session
    ? session.chatbox.state === "ready"
    : (currentSetupConfig.value?.osc.enabled ?? false);

  return {
    color: enabled ? ("success" as const) : ("neutral" as const),
    label: uiText(enabled ? "live.chatbox.on" : "live.chatbox.off"),
  };
});

const pendingSessionChangesDescription = computed(() =>
  uiText(
    currentSession.value?.phase === "error"
      ? "live.currentSetup.pendingChanges.failedDescription"
      : "live.currentSetup.pendingChanges.description",
  ),
);

const hasActiveSession = computed(() =>
  isActiveRuntimeSessionPhase(currentSession.value?.phase),
);

const currentPublication = computed(() =>
  publicationDisplayPlanView(
    sessionRuntimePlan.value,
    desiredRuntimePlan.value,
  ),
);

const currentPublicationLabel = computed(() => {
  const publication = currentPublication.value;

  if (publication.state === "unavailable") {
    return uiText("live.publication.unavailable");
  }

  const mode = uiText(publicationModeMessageKey[publication.mode]);

  if (publication.state === "incompatible") {
    return uiText("live.publication.incompatibleValue", { mode });
  }

  return uiText("live.publication.readyValue", {
    description: publicationPlanDescription(publication),
    mode,
  });
});

const isStartBlocked = computed(() =>
  publicationStartIsBlocked(hasActiveSession.value, desiredRuntimePlan.value),
);
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
        <UBadge
          v-if="currentSetupConfig && !currentSession"
          color="info"
          variant="subtle"
        >
          {{ uiText("live.currentSetup.nextStartBadge") }}
        </UBadge>
        <UBadge color="neutral" variant="subtle">
          {{
            currentSetupConfig
              ? uiText(sttProviderMessageKey[currentSetupConfig.stt.provider])
              : uiText("common.loading")
          }}
        </UBadge>
        <UBadge :color="chatboxBadge.color" variant="subtle">
          {{ chatboxBadge.label }}
        </UBadge>
      </div>
    </header>

    <UAlert
      v-if="pendingSessionChanges.length > 0"
      color="warning"
      icon="i-lucide-triangle-alert"
      :title="uiText('live.currentSetup.pendingChanges.title')"
      :description="pendingSessionChangesDescription"
      variant="subtle"
    />

    <UAlert
      v-if="isStartBlocked"
      color="error"
      icon="i-lucide-circle-alert"
      role="alert"
      :title="uiText('live.publication.blocked.title')"
      :description="uiText('live.publication.blocked.description')"
      variant="subtle"
    >
      <template #actions>
        <UButton
          color="neutral"
          :label="uiText('live.publication.blocked.action')"
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
        :error-message="runtimeError"
        :in-flight-command="inFlightRuntimeCommand"
        :is-busy="isRuntimeBusy"
        :is-start-blocked="isStartBlocked"
        :runtime-status="runtimeStatus"
        @run="runCommand"
      />

      <div class="grid gap-5">
        <LiveAudioMeter
          :generation="currentSession?.generation ?? null"
          :level="latestAudioLevel"
          :session-phase="currentSession?.phase ?? null"
        />

        <UCard :ui="{ body: 'p-5' }">
          <template #header>
            <div class="flex items-center justify-between gap-4">
              <h2 class="text-base font-semibold text-highlighted">
                {{
                  uiText(
                    currentSession?.phase === "error"
                      ? "live.currentSetup.failedSessionTitle"
                      : currentSession
                        ? "live.currentSetup.activeSessionTitle"
                        : currentSetupConfig
                          ? "live.currentSetup.nextStartTitle"
                          : "live.currentSetup.title",
                  )
                }}
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
                {{ currentSetupConfig?.stt.model ?? uiText("common.loading") }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("live.currentSetup.publication") }}
              </dt>
              <dd
                class="min-w-0 text-right font-medium break-words text-highlighted"
              >
                {{ currentPublicationLabel }}
              </dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-muted">
                {{ uiText("live.currentSetup.oscTarget") }}
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
                {{ uiText("live.recentActivity.latestCompletedCaption") }}
              </p>
              <p class="mt-1 leading-6 break-words text-highlighted">
                {{
                  latestCompletedCaption?.text ??
                  uiText("live.recentActivity.noCompletedCaption")
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

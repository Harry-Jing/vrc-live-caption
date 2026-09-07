<script setup lang="ts">
import { uiText } from "../../i18n/uiText";
import {
  translationFailureReasonMessageKey,
  translationPresentationStateColor,
  translationPresentationStateMessageKey,
  translationUnitStateColor,
  translationUnitStateMessageKey,
} from "../../runtime/presentation";
import type { TranslationPresentation } from "../../runtime/translationPresentation";

defineProps<{
  presentation: TranslationPresentation;
}>();

function unitKey(unit: TranslationPresentation["units"][number]) {
  const sourceRef = unit.sourceRef;

  return [
    sourceRef.generation,
    sourceRef.streamId,
    sourceRef.unitId,
    sourceRef.revision,
  ].join(":");
}
</script>

<template>
  <UCard data-testid="translation-activity" :ui="{ body: 'p-6 sm:p-7' }">
    <div
      class="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between"
    >
      <div>
        <h2 class="text-base font-semibold text-highlighted">
          {{ uiText("captioning.translationActivity.title") }}
        </h2>
        <p class="mt-1 max-w-3xl text-sm text-muted">
          {{ uiText("captioning.translationActivity.description") }}
        </p>
      </div>
      <UBadge
        :color="translationPresentationStateColor[presentation.state]"
        aria-atomic="true"
        aria-live="polite"
        role="status"
        variant="subtle"
      >
        {{ uiText(translationPresentationStateMessageKey[presentation.state]) }}
      </UBadge>
    </div>

    <template v-if="presentation.state !== 'inactive'">
      <UAlert
        v-if="presentation.state === 'degraded'"
        class="mt-5"
        color="warning"
        icon="i-lucide-triangle-alert"
        :title="uiText('captioning.translationActivity.status.degraded')"
        :description="`${uiText('captioning.translationActivity.degradedDescription')} ${uiText(translationFailureReasonMessageKey[presentation.reasonCode])}`"
        variant="subtle"
      />

      <p v-if="presentation.units.length === 0" class="mt-6 text-sm text-muted">
        {{ uiText("captioning.translationActivity.noUnits") }}
      </p>

      <ol
        v-else
        :aria-label="uiText('captioning.translationActivity.unitsLabel')"
        class="mt-6 grid gap-4"
      >
        <li
          v-for="unit in presentation.units"
          :key="unitKey(unit)"
          class="rounded-lg border border-default p-4"
        >
          <div class="flex items-center justify-between gap-4">
            <UBadge
              :color="translationUnitStateColor[unit.state]"
              variant="subtle"
            >
              {{ uiText(translationUnitStateMessageKey[unit.state]) }}
            </UBadge>
          </div>

          <div class="mt-4">
            <p class="text-xs font-semibold tracking-wide text-muted uppercase">
              {{ uiText("captioning.translationActivity.sourceLabel") }}
            </p>
            <p
              class="mt-1 leading-6 break-words text-highlighted"
              :lang="unit.source.language ?? undefined"
            >
              {{ unit.source.text }}
            </p>
          </div>

          <div v-if="unit.state === 'completed'" class="mt-4">
            <p class="text-xs font-semibold tracking-wide text-muted uppercase">
              {{ uiText("captioning.translationActivity.translationLabel") }}
            </p>
            <p
              class="mt-1 text-lg leading-7 break-words text-highlighted"
              :lang="unit.translation.language ?? undefined"
            >
              {{ unit.translation.text }}
            </p>
          </div>
          <p
            v-else-if="unit.state === 'pending'"
            class="mt-4 text-sm text-muted"
          >
            {{
              uiText("captioning.translationActivity.unit.pendingDescription")
            }}
          </p>
          <div v-else class="mt-4 text-sm">
            <p class="font-medium text-error">
              {{ uiText(translationFailureReasonMessageKey[unit.reasonCode]) }}
            </p>
            <p class="mt-1 text-muted">
              {{
                uiText(
                  presentation.content === "translationOnly"
                    ? "captioning.translationActivity.unit.failedTranslationOnly"
                    : "captioning.translationActivity.unit.failedBilingual",
                )
              }}
            </p>
          </div>
        </li>
      </ol>
    </template>
  </UCard>
</template>

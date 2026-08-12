<script setup lang="ts">
import type { UiLocale } from "../../i18n/uiLocale";
import { currentUiLocale } from "../../i18n/uiLocale";
import type { TranslationPresentation } from "../../runtime/translationPresentation";
import {
  translationActivityText,
  translationFailureText,
} from "./translationActivityText";

const props = withDefaults(
  defineProps<{
    presentation: TranslationPresentation;
    locale?: UiLocale;
  }>(),
  { locale: currentUiLocale },
);

const text = (key: Parameters<typeof translationActivityText>[1]) =>
  translationActivityText(props.locale, key);

function contentText(content: "translationOnly" | "bilingual") {
  return text(
    content === "translationOnly"
      ? "contentTranslationOnly"
      : "contentBilingual",
  );
}

function targetText(target: "en" | "zh-Hans") {
  return text(target === "en" ? "targetEnglish" : "targetSimplifiedChinese");
}

function endpointText(endpoint: "official" | "custom") {
  return text(endpoint === "official" ? "endpointOfficial" : "endpointCustom");
}

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
  <UCard
    data-testid="translation-activity"
    :lang="locale"
    :ui="{ body: 'p-6 sm:p-7' }"
  >
    <div
      class="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between"
    >
      <div>
        <h2 class="text-base font-semibold text-highlighted">
          {{ text("title") }}
        </h2>
        <p class="mt-1 max-w-3xl text-sm text-muted">
          {{ text("description") }}
        </p>
      </div>
      <UBadge
        :color="
          presentation.state === 'degraded'
            ? 'warning'
            : presentation.state === 'active'
              ? 'success'
              : 'neutral'
        "
        aria-atomic="true"
        aria-live="polite"
        role="status"
        variant="subtle"
      >
        {{
          text(
            presentation.state === "degraded"
              ? "degraded"
              : presentation.state === "active"
                ? "active"
                : "inactive",
          )
        }}
      </UBadge>
    </div>

    <template v-if="presentation.state !== 'inactive'">
      <dl class="mt-5 grid gap-3 text-sm sm:grid-cols-3">
        <div class="rounded-md border border-default bg-muted/30 p-3">
          <dt class="text-muted">{{ text("contentLabel") }}</dt>
          <dd class="mt-1 font-medium text-highlighted">
            {{ contentText(presentation.content) }}
          </dd>
        </div>
        <div class="rounded-md border border-default bg-muted/30 p-3">
          <dt class="text-muted">{{ text("targetLabel") }}</dt>
          <dd class="mt-1 font-medium text-highlighted">
            {{ targetText(presentation.target) }}
          </dd>
        </div>
        <div class="rounded-md border border-default bg-muted/30 p-3">
          <dt class="text-muted">{{ text("endpointLabel") }}</dt>
          <dd class="mt-1 font-medium text-highlighted">
            {{ endpointText(presentation.endpointKind) }}
          </dd>
        </div>
      </dl>

      <UAlert
        v-if="presentation.state === 'degraded'"
        class="mt-5"
        color="warning"
        icon="i-lucide-triangle-alert"
        :title="text('degraded')"
        :description="`${text('degradedDescription')} ${translationFailureText(locale, presentation.reasonCode)}`"
        variant="subtle"
      />

      <p v-if="presentation.units.length === 0" class="mt-6 text-sm text-muted">
        {{ text("noUnits") }}
      </p>

      <ol
        v-else
        :aria-label="text('unitsLabel')"
        aria-live="polite"
        class="mt-6 grid gap-4"
      >
        <li
          v-for="unit in presentation.units"
          :key="unitKey(unit)"
          class="rounded-lg border border-default p-4"
        >
          <div class="flex items-center justify-between gap-4">
            <UBadge
              :color="
                unit.state === 'completed'
                  ? 'success'
                  : unit.state === 'failed'
                    ? 'error'
                    : 'info'
              "
              variant="subtle"
            >
              {{
                text(
                  unit.state === "completed"
                    ? "completed"
                    : unit.state === "failed"
                      ? "failed"
                      : "pending",
                )
              }}
            </UBadge>
          </div>

          <div v-if="unit.source" class="mt-4">
            <p class="text-xs font-semibold tracking-wide text-muted uppercase">
              {{ text("sourceLabel") }}
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
              {{ text("translationLabel") }}
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
            {{ text("pendingDescription") }}
          </p>
          <div v-else class="mt-4 text-sm">
            <p
              v-if="presentation.content === 'translationOnly'"
              class="text-muted"
            >
              {{ text("failedTranslationOnly") }}
            </p>
            <p class="mt-1 font-medium text-error">
              {{ translationFailureText(locale, unit.reasonCode) }}
            </p>
          </div>
        </li>
      </ol>
    </template>
  </UCard>
</template>

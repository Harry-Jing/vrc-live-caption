<script setup lang="ts">
import { computed, ref } from "vue";
import type { CredentialStatus } from "../../runtime/runtimeControl";
import { credentialStatusPresentation } from "./credentialStatusPresentation";
import type { ServiceCredentialControlCopy } from "./serviceCredentialControl";

const props = defineProps<{
  actionFailure: string;
  busy: boolean;
  capturedByActiveGeneration: boolean;
  copy: ServiceCredentialControlCopy;
  status: CredentialStatus | null;
}>();

const emit = defineEmits<{
  delete: [];
  save: [secret: string];
}>();

const secretInput = ref("");
const isRemoveModalOpen = ref(false);
const presentation = computed(() => credentialStatusPresentation(props.status));
const canSave = computed(() => secretInput.value.trim().length > 0);
const saveLabel = computed(() =>
  presentation.value.isStoredByApp ? props.copy.replace : props.copy.save,
);
const removeDescription = computed(() =>
  props.capturedByActiveGeneration
    ? props.copy.removeDialogCurrentGenerationDescription
    : props.copy.removeDialogDescription,
);

function saveSecret() {
  if (props.busy || !canSave.value) {
    return;
  }

  const secret = secretInput.value;
  secretInput.value = "";
  emit("save", secret);
}

function requestDelete() {
  isRemoveModalOpen.value = true;
}

function closeRemoveModal() {
  isRemoveModalOpen.value = false;
}

function confirmDelete() {
  isRemoveModalOpen.value = false;
  emit("delete");
}
</script>

<template>
  <section class="grid gap-3 rounded-md border border-default bg-muted/30 p-3">
    <div class="flex items-center justify-between gap-3">
      <h4 class="text-sm font-medium text-highlighted">
        {{ copy.title }}
      </h4>
      <UBadge :color="presentation.color" variant="subtle">
        {{ presentation.label }}
      </UBadge>
    </div>

    <p class="text-sm text-muted">
      {{ copy.disclosure }}
    </p>

    <form
      class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-end"
      @submit.prevent="saveSecret"
    >
      <UFormField :label="copy.inputLabel">
        <UInput
          v-model="secretInput"
          autocapitalize="off"
          autocomplete="off"
          class="w-full"
          :disabled="busy"
          :placeholder="copy.inputPlaceholder"
          spellcheck="false"
          type="password"
          @keydown.enter.prevent.stop="saveSecret"
        />
      </UFormField>
      <UButton
        :disabled="busy || !canSave"
        icon="i-lucide-key-round"
        :label="saveLabel"
        type="submit"
        variant="subtle"
      />
      <UButton
        v-if="presentation.canRemove"
        :disabled="busy"
        color="error"
        icon="i-lucide-trash-2"
        :label="copy.remove"
        type="button"
        variant="ghost"
        @click="requestDelete"
      />
    </form>

    <UAlert
      v-if="actionFailure"
      color="error"
      icon="i-lucide-circle-alert"
      role="alert"
      :title="copy.actionFailed"
      :description="actionFailure"
      variant="subtle"
    />

    <p
      v-if="presentation.failureMessage"
      class="text-xs text-error"
      role="alert"
    >
      {{ presentation.failureMessage }}
    </p>
  </section>

  <UModal
    v-model:open="isRemoveModalOpen"
    :title="copy.removeDialogTitle"
    :description="removeDescription"
  >
    <template #footer>
      <div class="flex w-full justify-end gap-2">
        <UButton
          color="neutral"
          :label="copy.removeDialogCancel"
          type="button"
          variant="outline"
          @click="closeRemoveModal"
        />
        <UButton
          :disabled="busy"
          color="error"
          icon="i-lucide-trash-2"
          :label="copy.removeDialogConfirm"
          type="button"
          @click="confirmDelete"
        />
      </div>
    </template>
  </UModal>
</template>

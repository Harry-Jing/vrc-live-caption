<script setup lang="ts">
defineProps<{
  mode: "partial" | "final";
  text: string;
}>();
</script>

<template>
  <UCard :ui="{ body: 'p-6 sm:p-7' }">
    <div class="mb-4 flex items-center justify-between gap-4">
      <div>
        <p class="text-xs font-semibold tracking-wide text-muted uppercase">
          Caption Preview
        </p>
        <h2 class="text-base font-semibold text-highlighted">Current output</h2>
      </div>
      <UBadge
        :color="mode === 'partial' ? 'warning' : 'success'"
        variant="subtle"
      >
        {{ mode }}
      </UBadge>
    </div>

    <p class="min-h-24 text-2xl leading-relaxed text-highlighted sm:text-3xl">
      {{ text }}
    </p>
  </UCard>
</template>

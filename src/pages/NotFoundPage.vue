<script setup lang="ts">
import { RouterLink } from "vue-router";
</script>

<template>
  <div class="grid min-h-[60dvh] place-items-center">
    <UCard :ui="{ body: 'p-6' }" class="w-full max-w-md">
      <div class="grid gap-4 text-center">
        <div>
          <p class="text-xs font-semibold tracking-wide text-muted uppercase">
            Page not found
          </p>
          <h2 class="mt-1 text-xl font-semibold text-highlighted">
            This route does not exist
          </h2>
        </div>

        <RouterLink
          class="mx-auto inline-flex min-h-11 items-center justify-center rounded-md bg-primary px-4 text-sm font-medium text-inverted outline-none hover:bg-primary-600 focus-visible:ring-2 focus-visible:ring-primary"
          to="/"
        >
          Back to Live
        </RouterLink>
      </div>
    </UCard>
  </div>
</template>

<script setup lang="ts">
import { useRuntime } from "./runtime/useRuntime";
import { provideRuntimeContext } from "./runtime/context";
import AppNavigation from "./components/AppNavigation.vue";

const runtime = useRuntime();

provideRuntimeContext(runtime);
</script>

<template>
  <UApp>
    <div class="min-h-dvh bg-muted text-default">
      <div
        class="mx-auto grid w-full max-w-7xl gap-5 px-4 py-5 sm:px-6 lg:grid-cols-[220px_minmax(0,1fr)] lg:px-8"
      >
        <AppNavigation :runtime-status="runtime.runtimeStatus.value" />

        <main class="min-w-0">
          <RouterView />
        </main>
      </div>
    </div>
  </UApp>
</template>

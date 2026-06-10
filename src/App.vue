<script setup lang="ts">
import { useRuntime } from "./runtime/useRuntime";
import { provideRuntimeContext } from "./runtime/context";
import AppNavigation from "./components/AppNavigation.vue";

const runtime = useRuntime();

provideRuntimeContext(runtime);
</script>

<template>
  <UApp>
    <main
      class="min-h-dvh bg-slate-100 text-slate-950 dark:bg-slate-950 dark:text-slate-50"
    >
      <div
        class="mx-auto grid w-full max-w-7xl gap-5 px-4 py-5 sm:px-6 lg:grid-cols-[220px_minmax(0,1fr)] lg:px-8"
      >
        <AppNavigation :runtime-status="runtime.runtimeStatus.value" />

        <section class="min-w-0">
          <RouterView />
        </section>
      </div>
    </main>
  </UApp>
</template>

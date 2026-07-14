<script setup lang="ts">
import { runtimeStatusColor } from "../runtime/presentation";
import type { RuntimeStatusEvent } from "../runtime/types";

defineProps<{
  runtimeStatus: RuntimeStatusEvent;
}>();

const navItems = [
  {
    label: "Live",
    to: "/",
    icon: "i-lucide-radio-tower",
  },
  {
    label: "Settings",
    to: "/settings",
    icon: "i-lucide-sliders-horizontal",
  },
  {
    label: "Diagnostics",
    to: "/diagnostics",
    icon: "i-lucide-activity",
  },
] as const;
</script>

<template>
  <aside class="grid gap-4 self-start lg:sticky lg:top-5">
    <div class="rounded-md border border-default bg-default p-4">
      <p class="text-xs font-semibold tracking-wide text-muted uppercase">
        VRC Live Caption
      </p>
      <h1 class="mt-1 text-lg font-semibold text-highlighted">
        Outgoing Caption
      </h1>

      <div class="mt-4 flex items-center justify-between gap-3">
        <span class="text-sm text-muted">Runtime</span>
        <UBadge
          :color="runtimeStatusColor[runtimeStatus.status]"
          variant="subtle"
          class="capitalize"
        >
          {{ runtimeStatus.status }}
        </UBadge>
      </div>
    </div>

    <nav
      aria-label="Primary"
      class="grid grid-cols-3 gap-2 rounded-md border border-default bg-default p-2 lg:grid-cols-1"
    >
      <RouterLink
        v-for="item in navItems"
        :key="item.to"
        v-slot="{ href, navigate, isActive }"
        custom
        :to="item.to"
      >
        <a
          :aria-current="isActive ? 'page' : undefined"
          :class="[
            'flex min-h-11 items-center justify-center gap-2 rounded-md px-3 text-sm font-medium transition outline-none focus-visible:ring-2 focus-visible:ring-primary lg:justify-start',
            isActive
              ? 'bg-primary text-inverted'
              : 'text-muted hover:bg-muted hover:text-highlighted',
          ]"
          :href="href"
          @click="navigate"
        >
          <UIcon :name="item.icon" class="size-4 shrink-0" aria-hidden="true" />
          <span>{{ item.label }}</span>
        </a>
      </RouterLink>
    </nav>
  </aside>
</template>

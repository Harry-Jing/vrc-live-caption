<template>
  <div class="grid min-h-[60dvh] place-items-center">
    <UCard :ui="{ body: 'p-6' }" class="w-full max-w-md">
      <div class="grid gap-4 text-center">
        <div>
          <p class="text-xs font-semibold tracking-wide text-muted uppercase">
            Page not found
          </p>
          <h1 class="mt-1 text-xl font-semibold text-highlighted">
            This route does not exist
          </h1>
        </div>

        <UButton class="mx-auto" label="Back to Live" to="/" />
      </div>
    </UCard>
  </div>
</template>

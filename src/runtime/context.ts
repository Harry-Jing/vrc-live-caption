import { inject, provide, type InjectionKey } from "vue";
import type { useRuntime } from "./useRuntime";

export type RuntimeContext = ReturnType<typeof useRuntime>;

const runtimeContextKey: InjectionKey<RuntimeContext> =
  Symbol("RuntimeContext");

export function provideRuntimeContext(runtime: RuntimeContext) {
  provide(runtimeContextKey, runtime);
}

export function useRuntimeContext() {
  const runtime = inject(runtimeContextKey);

  if (!runtime) {
    throw new Error("Runtime context is not available.");
  }

  return runtime;
}

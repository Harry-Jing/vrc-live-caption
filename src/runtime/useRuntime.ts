import { onBeforeUnmount, onMounted } from "vue";
import { createRuntimeBackend } from "../platform/runtimeBackend";
import { createRuntimeStore } from "./store/runtimeStore";

export function useRuntime() {
  const store = createRuntimeStore(createRuntimeBackend());

  onMounted(() => store.connect());
  onBeforeUnmount(() => {
    store.dispose();
  });

  return store.runtime;
}

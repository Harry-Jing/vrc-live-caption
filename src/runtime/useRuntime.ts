import { onBeforeUnmount, onMounted } from "vue";
import { createAppGateway } from "../platform/appGateway";
import { createRuntimeStore } from "./store/runtimeStore";

export function useRuntime() {
  const store = createRuntimeStore(createAppGateway());

  onMounted(() => store.connect());
  onBeforeUnmount(() => {
    store.dispose();
  });

  return store.runtime;
}

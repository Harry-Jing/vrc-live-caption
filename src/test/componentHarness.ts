import { createSSRApp, type Component } from "vue";
import { renderToString } from "@vue/server-renderer";

export async function renderComponent(
  component: Component,
  props: Record<string, unknown>,
) {
  const app = createSSRApp(component, props);

  return renderToString(app);
}

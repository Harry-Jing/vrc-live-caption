// @vitest-environment happy-dom

import ui from "@nuxt/ui/vue-plugin";
import { mount, type VueWrapper } from "@vue/test-utils";
import { defineComponent, h } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, describe, expect, test } from "vitest";
import CaptionPreview from "../features/captioning/CaptionPreview.vue";
import CaptioningPage from "../features/captioning/CaptioningPage.vue";
import TranslationActivity from "../features/captioning/TranslationActivity.vue";
import { provideRuntimeContext } from "../runtime/context";
import { createRuntimeStore } from "../runtime/store/runtimeStore";
import { createPreviewAppGateway } from "./preview/appGateway";

const mountedWrappers: VueWrapper[] = [];
const runtimeStores: ReturnType<typeof createRuntimeStore>[] = [];

afterEach(() => {
  for (const wrapper of mountedWrappers.splice(0)) {
    wrapper.unmount();
  }
  for (const store of runtimeStores.splice(0)) {
    store.dispose();
  }
});

async function mountPage(search = "") {
  const store = createRuntimeStore(createPreviewAppGateway(search));
  runtimeStores.push(store);
  await store.connect();

  const Harness = defineComponent({
    setup() {
      provideRuntimeContext(store.runtime);
      return () => h(CaptioningPage);
    },
  });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", component: Harness },
      { path: "/settings", component: Harness },
      { path: "/diagnostics", component: Harness },
    ],
  });
  await router.push("/");
  await router.isReady();
  const wrapper = mount(Harness, {
    attachTo: document.body,
    global: { plugins: [ui, router] },
  });
  mountedWrappers.push(wrapper);

  return wrapper;
}

describe("CaptioningPage Translation presentation", () => {
  test("leaves the current Source-only preview unchanged", async () => {
    const wrapper = await mountPage();

    expect(wrapper.findComponent(CaptionPreview).exists()).toBe(true);
    expect(wrapper.findComponent(TranslationActivity).exists()).toBe(false);
    expect(wrapper.text()).toContain("Caption Preview");
  });

  test("shows stopped Translation without falling back to stale Source activity", async () => {
    const wrapper = await mountPage("?translationScenario=stopped");

    expect(wrapper.findComponent(TranslationActivity).exists()).toBe(true);
    expect(wrapper.findComponent(CaptionPreview).exists()).toBe(false);
    expect(wrapper.text()).toContain("Translation stopped");
    expect(wrapper.text()).not.toContain(
      "Deterministic Source for official-stopped.",
    );
  });

  test("does not expose Source anywhere on a Translation-only failed page", async () => {
    const wrapper = await mountPage("?translationScenario=official-failed");

    expect(wrapper.text()).toContain("Translation failed");
    expect(wrapper.text()).toContain(
      "The Translation service rate-limited this request.",
    );
    expect(wrapper.text()).not.toContain(
      "Deterministic Source for official-failed-unit.",
    );
  });
});

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
    expect(wrapper.text()).toContain("Source only");
    expect(wrapper.text()).not.toContain("Translation activity");
  });

  test("keeps the Caption Preview above Translation activity during a bilingual run", async () => {
    const wrapper = await mountPage("?translationScenario=official-success");
    const preview = wrapper.findComponent(CaptionPreview);
    const activity = wrapper.findComponent(TranslationActivity);

    expect(preview.exists()).toBe(true);
    expect(activity.exists()).toBe(true);
    expect(
      preview.element.compareDocumentPosition(activity.element) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    expect(wrapper.text()).toContain(
      "Bilingual · Simplified Chinese (zh-Hans) · Official OpenAI",
    );
    expect(wrapper.text()).toContain("Translation active");
    expect(wrapper.text()).toContain("第 7 代的确定性译文。");
  });

  test("hides Translation activity after Stop while the next Start setup stays visible", async () => {
    const wrapper = await mountPage("?translationScenario=stopped");

    expect(wrapper.findComponent(CaptionPreview).exists()).toBe(true);
    expect(wrapper.findComponent(TranslationActivity).exists()).toBe(false);
    expect(wrapper.text()).toContain(
      "Translation only · Simplified Chinese (zh-Hans) · Official OpenAI",
    );
    expect(wrapper.text()).not.toContain("Translated");
    expect(wrapper.text()).not.toContain("Translation failed");
  });

  test("shows the failed Source caption and its Chatbox consequence on a Translation-only page", async () => {
    const wrapper = await mountPage("?translationScenario=official-failed");

    expect(wrapper.findComponent(CaptionPreview).exists()).toBe(true);
    expect(wrapper.text()).toContain("Translation failed");
    expect(wrapper.text()).toContain(
      "The Translation service rate-limited this request.",
    );
    expect(wrapper.text()).toContain(
      "Deterministic Source for official-failed-unit.",
    );
    expect(wrapper.text()).toContain("Chatbox skips this caption.");
  });
});

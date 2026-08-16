// @vitest-environment happy-dom

import ui from "@nuxt/ui/vue-plugin";
import { mount } from "@vue/test-utils";
import { expect, test } from "vitest";
import CompletedTranslationSettings from "./CompletedTranslationSettings.vue";

const baseProps: InstanceType<typeof CompletedTranslationSettings>["$props"] = {
  content: "sourceOnly",
  disabled: false,
  issues: { target: null, customApiBaseUrl: null },
  openAiCredentialStatus: { state: "unconfigured", id: "openai" },
  customCredentialStatus: {
    state: "unconfigured",
    id: "customTranslation",
  },
  publicationMode: "completed",
  translation: null,
};

function mountSettings(
  overrides: Partial<
    InstanceType<typeof CompletedTranslationSettings>["$props"]
  > = {},
) {
  return mount(CompletedTranslationSettings, {
    props: { ...baseProps, ...overrides },
    global: { plugins: [ui] },
    attachTo: document.body,
  });
}

test("keeps dormant Translation controls and upload disclosures hidden for Source-only", () => {
  const wrapper = mountSettings({
    translation: {
      target: "zh-Hans",
      endpointKind: "custom",
      customApiBaseUrl: "https://example.com/v1",
    },
  });

  expect(wrapper.text()).toContain("Translation inactive");
  expect(wrapper.text()).toContain("next Start");
  expect(wrapper.text()).not.toContain("Completed Source text is sent");
  expect(wrapper.text()).not.toContain("Custom API base URL");
  expect(wrapper.text()).not.toContain("Custom Translation credentials");

  wrapper.unmount();
});

test("shows explicit target, Official disclosure, credential state, and next-Start semantics", () => {
  const wrapper = mountSettings({
    content: "bilingual",
    openAiCredentialStatus: {
      state: "configured",
      id: "openai",
      storage: "environment",
      displaySuffix: "abcd",
    },
    translation: {
      target: "zh-Hans",
      endpointKind: "official",
      customApiBaseUrl: "",
    },
  });

  expect(wrapper.text()).toContain("Caption content");
  expect(wrapper.text()).toContain("Translation target");
  expect(wrapper.text()).toContain("English (en)");
  expect(wrapper.text()).toContain("Simplified Chinese (zh-Hans)");
  expect(wrapper.text()).toContain("Translation endpoint");
  expect(wrapper.text()).toContain("Completed Source text is sent to OpenAI");
  expect(wrapper.text()).toContain("store: false");
  expect(wrapper.text()).toContain("Environment ...abcd");
  expect(wrapper.text()).toContain("next Start");

  wrapper.unmount();
});

test("shows Custom URL validation, separate credential status, and operator retention disclosure", () => {
  const wrapper = mountSettings({
    content: "translationOnly",
    customCredentialStatus: {
      state: "configured",
      id: "customTranslation",
      storage: "systemCredentialStore",
      displaySuffix: "wxyz",
    },
    issues: {
      target: null,
      customApiBaseUrl: "httpsRequired",
    },
    translation: {
      target: "en",
      endpointKind: "custom",
      customApiBaseUrl: "http://example.com/v1",
    },
  });

  expect(wrapper.text()).toContain("Custom API base URL");
  expect(wrapper.text()).toContain("API base URL must use HTTPS.");
  expect(wrapper.text()).toContain("System ...wxyz");
  expect(wrapper.text()).toContain("operator's retention policy");
  expect(wrapper.text()).toContain("Recognition audio is not rerouted");

  wrapper.unmount();
});

test("radio groups support arrow-key selection without inferring a target", async () => {
  const wrapper = mountSettings();
  const contentRadios = wrapper.findAll('[role="radio"]');

  expect(contentRadios).toHaveLength(3);
  await contentRadios[0]?.trigger("focus");
  await new Promise((resolve) => setTimeout(resolve, 0));
  await contentRadios[0]?.trigger("keydown", {
    key: "ArrowRight",
    code: "ArrowRight",
  });
  await new Promise((resolve) => setTimeout(resolve, 0));

  expect(wrapper.emitted("selectContent")).toEqual([["translationOnly"]]);
  expect(wrapper.emitted("selectTarget")).toBeUndefined();

  wrapper.unmount();
});

test("supports keyboard selection for an explicitly unselected target", async () => {
  const wrapper = mountSettings({
    content: "bilingual",
    issues: { target: "required", customApiBaseUrl: null },
    translation: {
      target: null,
      endpointKind: "official",
      customApiBaseUrl: "",
    },
  });
  const targetGroup = wrapper.findAll('[role="radiogroup"]')[1];
  const targetRadios = targetGroup?.findAll('[role="radio"]') ?? [];

  expect(targetRadios).toHaveLength(2);
  expect(wrapper.emitted("selectTarget")).toBeUndefined();
  await targetRadios[0]?.trigger("focus");
  await new Promise((resolve) => setTimeout(resolve, 0));
  await targetRadios[0]?.trigger("keydown", {
    key: "ArrowRight",
    code: "ArrowRight",
  });
  await new Promise((resolve) => setTimeout(resolve, 0));

  expect(wrapper.emitted("selectTarget")).toEqual([["zh-Hans"]]);

  wrapper.unmount();
});

test("offers an explicit Completed recovery for Live Translation content", async () => {
  const wrapper = mountSettings({
    content: "bilingual",
    publicationMode: "live",
    translation: {
      target: "en",
      endpointKind: "official",
      customApiBaseUrl: "",
    },
  });

  expect(wrapper.text()).toContain("Translation content requires Completed");
  const action = wrapper
    .findAll("button")
    .find((button) => button.text().includes("Use Completed"));
  if (!action) {
    throw new Error("Completed recovery action was not rendered.");
  }
  await action.trigger("click");
  expect(wrapper.emitted("useCompleted")).toEqual([[]]);

  await wrapper.setProps({ disabled: true });
  expect(action.attributes("disabled")).toBeDefined();

  wrapper.unmount();
});

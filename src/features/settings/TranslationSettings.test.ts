// @vitest-environment happy-dom

import ui from "@nuxt/ui/vue-plugin";
import { mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { afterEach, describe, expect, test } from "vitest";
import TranslationSettings from "./TranslationSettings.vue";
import type { TranslationSettingsDraft } from "./translationSettingsModel";

const officialUnconfigured = {
  state: "unconfigured",
  id: "openai",
} as const;
const customUnconfigured = {
  state: "unconfigured",
  id: "customTranslation",
} as const;
const mountedWrappers: VueWrapper[] = [];

afterEach(() => {
  for (const wrapper of mountedWrappers.splice(0)) {
    wrapper.unmount();
  }
});

function draft(
  overrides: Partial<TranslationSettingsDraft> = {},
): TranslationSettingsDraft {
  return {
    content: "sourceOnly",
    target: null,
    endpointKind: "official",
    customApiBaseUrl: "",
    ...overrides,
  };
}

function mountSettings(
  modelValue: TranslationSettingsDraft,
  overrides: Record<string, unknown> = {},
) {
  const wrapper = mount(TranslationSettings, {
    attachTo: document.body,
    props: {
      modelValue,
      officialCredentialStatus: officialUnconfigured,
      customCredentialStatus: customUnconfigured,
      locale: "en",
      "onUpdate:modelValue": async (value: TranslationSettingsDraft) => {
        await wrapper.setProps({ modelValue: value });
      },
      ...overrides,
    },
    global: { plugins: [ui] },
  });
  mountedWrappers.push(wrapper);

  return wrapper;
}

describe("TranslationSettings", () => {
  test("keeps Source-only dormant without showing an upload disclosure", () => {
    const wrapper = mountSettings(
      draft({
        target: "zh-Hans",
        endpointKind: "custom",
        customApiBaseUrl: "https://translation.example.test/v1",
      }),
    );

    expect(wrapper.text()).toContain("Translation is dormant");
    expect(wrapper.text()).toContain("does not use a Translation credential");
    expect(wrapper.text()).not.toContain("Completed Source text is sent");
    expect(wrapper.text()).toContain("Simplified Chinese");
    expect(
      (wrapper.get('input[type="url"]').element as HTMLInputElement).value,
    ).toBe("https://translation.example.test/v1");
  });

  test("requires an explicit target when selected content includes Translation", () => {
    const wrapper = mountSettings(draft({ content: "bilingual" }));

    expect(wrapper.text()).toContain("Choose English or Simplified Chinese.");
    expect(wrapper.text()).toContain(
      "never infers this from the UI language, recognition hints, or Source text",
    );
    expect(wrapper.text()).toContain("Completed Source text is sent to OpenAI");
  });

  test("requires a complete target before preserving a dormant Custom selection", () => {
    const wrapper = mountSettings(
      draft({
        endpointKind: "custom",
        customApiBaseUrl: "https://operator.example.test/v1",
      }),
    );

    expect(wrapper.text()).toContain("Choose English or Simplified Chinese.");
    expect(
      wrapper
        .get('[data-testid="translation-target"]')
        .attributes("aria-required"),
    ).toBe("true");
  });

  test("shows Custom URL validation, retention disclosure, and separate credentials", () => {
    const wrapper = mountSettings(
      draft({
        content: "translationOnly",
        target: "en",
        endpointKind: "custom",
        customApiBaseUrl: "http://operator.example.test/v1",
      }),
    );

    expect(wrapper.text()).toContain("The API base URL must use HTTPS.");
    expect(wrapper.text()).toContain(
      "does not define the operator's retention policy",
    );
    expect(wrapper.text()).toContain("Custom Translation credential");
    expect(wrapper.text()).toContain("Stored separately");
  });

  test("uses the existing OpenAI environment credential for Official Translation", () => {
    const wrapper = mountSettings(draft({ target: "en" }), {
      officialCredentialStatus: {
        state: "configured",
        id: "openai",
        storage: "environment",
        displaySuffix: "test",
      },
    });

    expect(wrapper.text()).toContain("Provided by environment");
    expect(wrapper.text()).toContain("reuses the OpenAI credential");
    expect(wrapper.text()).not.toContain("Custom endpoint key");
  });

  test("shows unavailable Custom credential failures without secret values", () => {
    const wrapper = mountSettings(
      draft({
        target: "en",
        endpointKind: "custom",
        customApiBaseUrl: "https://operator.example.test/v1",
      }),
      {
        customCredentialStatus: {
          state: "unavailable",
          id: "customTranslation",
          failure: {
            code: "config.secret_failed",
            message: "System credential store is unavailable.",
          },
        },
      },
    );

    expect(wrapper.text()).toContain("Unavailable");
    expect(wrapper.text()).toContain("System credential store is unavailable.");
    expect(wrapper.html()).toContain('role="alert"');
  });

  test("sets only the Custom credential, then clears plaintext", async () => {
    const wrapper = mountSettings(
      draft({
        target: "en",
        endpointKind: "custom",
        customApiBaseUrl: "https://operator.example.test/v1",
      }),
    );
    const input = wrapper.get('input[type="password"]');

    await input.setValue("new-custom-secret");
    const saveButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("Save key"));
    if (!saveButton) {
      throw new Error("Expected the Custom credential save button.");
    }
    await saveButton.trigger("click");
    await nextTick();

    expect(wrapper.emitted("saveCredential")).toEqual([
      ["customTranslation", "new-custom-secret"],
    ]);
    expect((input.element as HTMLInputElement).value).toBe("");
    expect(wrapper.html()).not.toContain("new-custom-secret");
  });

  test("replaces only the Custom credential, then clears plaintext", async () => {
    const wrapper = mountSettings(
      draft({
        target: "en",
        endpointKind: "custom",
        customApiBaseUrl: "https://operator.example.test/v1",
      }),
      {
        customCredentialStatus: {
          state: "configured",
          id: "customTranslation",
          storage: "systemCredentialStore",
          displaySuffix: "1234",
        },
      },
    );
    const input = wrapper.get('input[type="password"]');

    await input.setValue("custom-secret-value");
    const replaceButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("Replace key"));
    if (!replaceButton) {
      throw new Error("Expected the Custom credential replace button.");
    }
    await replaceButton.trigger("click");
    await nextTick();

    expect(wrapper.emitted("saveCredential")).toEqual([
      ["customTranslation", "custom-secret-value"],
    ]);
    expect((input.element as HTMLInputElement).value).toBe("");
    expect(wrapper.html()).not.toContain("custom-secret-value");
    expect(wrapper.text()).toContain("Replace key");
    expect(wrapper.text()).toContain("Remove key");
  });

  test("announces immutable current-run behavior while the runtime is active", () => {
    const wrapper = mountSettings(draft({ target: "en" }), {
      showNextStartDisclosure: true,
    });

    expect(wrapper.text()).toContain("Applies on the next Start");
    expect(wrapper.text()).toContain("Saving does not change the current run");
  });

  test("confirms removal and identifies a credential captured by the current run", async () => {
    const wrapper = mountSettings(
      draft({
        target: "en",
        endpointKind: "custom",
        customApiBaseUrl: "https://operator.example.test/v1",
      }),
      {
        customCredentialCaptured: true,
        customCredentialStatus: {
          state: "configured",
          id: "customTranslation",
          storage: "systemCredentialStore",
          displaySuffix: "1234",
        },
      },
    );
    const removeButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("Remove key"));
    if (!removeButton) {
      throw new Error("Expected the Custom credential remove button.");
    }

    await removeButton.trigger("click");
    await nextTick();
    expect(document.body.textContent).toContain(
      "current run keeps the credential captured at Start",
    );

    const confirmButtons = Array.from(
      document.body.querySelectorAll("button"),
    ).filter((button) => button.textContent.includes("Remove key"));
    const confirmButton = confirmButtons.at(-1);
    if (!confirmButton) {
      throw new Error("Expected the Custom credential confirmation button.");
    }
    confirmButton.click();
    await nextTick();

    expect(wrapper.emitted("deleteCredential")).toEqual([
      ["customTranslation"],
    ]);
  });

  test("renders every control and disclosure in Simplified Chinese", () => {
    const wrapper = mountSettings(
      draft({
        content: "bilingual",
        target: "zh-Hans",
        endpointKind: "custom",
        customApiBaseUrl: "https://operator.example.test/v1",
      }),
      { locale: "zh-Hans", showNextStartDisclosure: true },
    );

    expect(wrapper.text()).toContain("已完成内容");
    expect(wrapper.text()).toContain("翻译目标语言");
    expect(wrapper.text()).toContain("自定义 API 基础 URL");
    expect(wrapper.text()).toContain("数据保留策略");
    expect(wrapper.text()).toContain("下次启动时生效");
  });

  test("localizes Custom URL validation in Simplified Chinese", () => {
    const wrapper = mountSettings(
      draft({
        content: "translationOnly",
        target: "en",
        endpointKind: "custom",
        customApiBaseUrl: "http://operator.example.test/v1",
      }),
      { locale: "zh-Hans" },
    );

    expect(wrapper.text()).toContain("API 基础 URL 必须使用 HTTPS。");
    expect(wrapper.text()).not.toContain("must use HTTPS");
  });

  test("supports arrow-key operation for the content radio group", async () => {
    const wrapper = mountSettings(draft());
    const contentRadios = wrapper.findAll(
      '[data-testid="translation-content"] [role="radio"]',
    );

    expect(contentRadios).toHaveLength(3);
    const sourceOnly = contentRadios[0];
    if (!sourceOnly) {
      throw new Error("Expected the Source-only radio.");
    }
    (sourceOnly.element as HTMLElement).focus();
    await new Promise((resolve) => window.setTimeout(resolve, 0));
    await sourceOnly.trigger("keydown", { key: "ArrowRight" });
    await new Promise((resolve) => window.setTimeout(resolve, 20));
    await nextTick();

    expect(wrapper.emitted("update:modelValue")?.at(-1)?.[0]).toMatchObject({
      content: "translationOnly",
    });
  });

  test("supports arrow-key operation for target and endpoint radio groups", async () => {
    const wrapper = mountSettings(
      draft({ content: "bilingual", target: "en" }),
    );

    for (const [testId, expected] of [
      ["translation-target", { target: "zh-Hans" }],
      ["translation-endpoint", { endpointKind: "custom" }],
    ] as const) {
      const radios = wrapper.findAll(
        `[data-testid="${testId}"] [role="radio"]`,
      );
      const first = radios[0];
      if (!first) {
        throw new Error(`Expected radio controls for ${testId}.`);
      }

      (first.element as HTMLElement).focus();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
      await first.trigger("keydown", { key: "ArrowRight" });
      await new Promise((resolve) => window.setTimeout(resolve, 20));
      await nextTick();

      expect(wrapper.emitted("update:modelValue")?.at(-1)?.[0]).toMatchObject(
        expected,
      );
    }
  });
});

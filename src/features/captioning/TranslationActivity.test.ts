// @vitest-environment happy-dom

import ui from "@nuxt/ui/vue-plugin";
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, describe, expect, test } from "vitest";
import TranslationActivity from "./TranslationActivity.vue";
import type {
  TranslationPresentation,
  TranslationPresentationUnit,
} from "../../runtime/translationPresentation";

const failedSourceRef = {
  generation: 7,
  streamId: "recognition-7-1",
  unitId: "failed",
  revision: 1,
} as const;
const pendingSourceRef = {
  generation: 7,
  streamId: "recognition-7-1",
  unitId: "pending",
  revision: 2,
} as const;
const completedSourceRef = {
  generation: 7,
  streamId: "recognition-7-1",
  unitId: "completed",
  revision: 3,
} as const;
const bilingualUnits: readonly TranslationPresentationUnit[] = [
  {
    state: "failed",
    sourceRef: failedSourceRef,
    source: { text: "Source whose Translation failed.", language: "en" },
    translation: null,
    reasonCode: "translation.provider_rate_limited",
  },
  {
    state: "pending",
    sourceRef: pendingSourceRef,
    source: { text: "Source awaiting Translation.", language: "en" },
    translation: null,
    reasonCode: null,
  },
  {
    state: "completed",
    sourceRef: completedSourceRef,
    source: { text: "Source with Translation.", language: "en" },
    translation: { text: "已完成的译文。", language: "zh-Hans" },
    reasonCode: null,
  },
];
const mountedWrappers: VueWrapper[] = [];

afterEach(() => {
  for (const wrapper of mountedWrappers.splice(0)) {
    wrapper.unmount();
  }
});

function presentation(
  overrides: Partial<
    Extract<TranslationPresentation, { state: "active" | "degraded" }>
  > = {},
): Extract<TranslationPresentation, { state: "active" | "degraded" }> {
  return {
    state: "active",
    content: "bilingual",
    target: "zh-Hans",
    endpointKind: "official",
    reasonCode: null,
    units: bilingualUnits,
    ...overrides,
  } as Extract<TranslationPresentation, { state: "active" | "degraded" }>;
}

function mountActivity(value: TranslationPresentation) {
  const wrapper = mount(TranslationActivity, {
    attachTo: document.body,
    props: { presentation: value },
    global: { plugins: [ui] },
  });
  mountedWrappers.push(wrapper);

  return wrapper;
}

describe("TranslationActivity", () => {
  test("renders exact bilingual progress and terminal states as an accessible list", () => {
    const wrapper = mountActivity(presentation());
    const units = wrapper.findAll("li");
    const failed = units.at(0);
    const pending = units.at(1);
    const completed = units.at(2);
    if (!failed || !pending || !completed) {
      throw new Error("Expected one failed, pending, and completed unit.");
    }

    expect(wrapper.get("ol").attributes("aria-label")).toBe(
      "Translation for the current run",
    );
    expect(wrapper.get("ol").attributes("aria-live")).toBeUndefined();
    expect(units).toHaveLength(3);
    expect(failed.text()).toContain("Source whose Translation failed.");
    expect(failed.text()).toContain(
      "The Translation service rate-limited this request.",
    );
    expect(failed.text()).toContain(
      "Chatbox shows only the Source for this caption.",
    );
    expect(pending.text()).toContain("Translating");
    expect(pending.text()).toContain(
      "Waiting for the exact Translation to finish.",
    );
    expect(completed.text()).toContain("Source with Translation.");
    expect(completed.text()).toContain("已完成的译文。");
    expect(completed.get('p[lang="en"]').text()).toBe(
      "Source with Translation.",
    );
    expect(completed.get('p[lang="zh-Hans"]').text()).toBe("已完成的译文。");
  });

  test("keeps Source visible in Translation-only and states the Chatbox consequence", () => {
    const wrapper = mountActivity(
      presentation({
        content: "translationOnly",
        endpointKind: "custom",
      }),
    );
    const rendered = wrapper.text();

    expect(rendered).toContain("Source whose Translation failed.");
    expect(rendered).toContain("Chatbox skips this caption.");
    expect(rendered).not.toContain("Chatbox shows only the Source");
    expect(rendered).toContain("Source awaiting Translation.");
    expect(rendered).toContain("Source with Translation.");
    expect(wrapper.html()).not.toMatch(/https?:|api[_ -]?key|bearer/iu);
  });

  test("renders degraded status and stable failure reasons without provider payloads", () => {
    const wrapper = mountActivity(
      presentation({
        state: "degraded",
        reasonCode: "translation.provider_unavailable",
      }),
    );

    expect(wrapper.text()).toContain("Translation degraded");
    expect(wrapper.text()).toContain(
      "The Translation service was unavailable.",
    );
    expect(wrapper.text()).toContain(
      "Recognition and the selected content continue unchanged.",
    );
    expect(wrapper.text()).not.toContain("translation.provider_unavailable");
  });

  test("renders a stopped Translation selection without stale units", () => {
    const wrapper = mountActivity({
      state: "inactive",
      content: null,
      target: null,
      endpointKind: null,
      reasonCode: null,
      units: [],
    });

    expect(wrapper.text()).toContain("Translation inactive");
    expect(wrapper.find("ol").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Source whose Translation failed.");
  });
});

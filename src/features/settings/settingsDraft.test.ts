import { nextTick, ref } from "vue";
import { expect, test } from "vitest";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
} from "../../runtime/appConfig";
import { useSettingsDraft } from "./settingsDraft";

function appConfig(): AppConfig {
  return {
    schemaVersion: APP_CONFIG_SCHEMA_VERSION,
    audio: { inputDeviceId: null },
    recognition: {
      path: "openai/gpt-live-transcribe",
      expectedLanguages: ["zh", "en"],
    },
    translation: null,
    osc: { host: "127.0.0.1", port: 9_000, enabled: true },
    publication: { mode: "live", content: "sourceOnly" },
    ui: { showOngoingPreview: true },
  };
}

test("has no editable or saveable draft until saved config exists", async () => {
  const saved = ref<AppConfig | null>(null);
  const settings = useSettingsDraft(() => saved.value);

  expect(settings.draft.value).toBeNull();
  expect(settings.isDirty.value).toBe(false);
  expect(settings.hasValidExpectedLanguages.value).toBe(false);
  expect(settings.createSaveConfig()).toBeNull();

  saved.value = appConfig();
  await nextTick();

  expect(settings.draft.value).not.toBe(saved.value);
  expect(settings.isDirty.value).toBe(false);
});

test("keeps a dirty cloned draft until saved config content changes", async () => {
  const saved = ref<AppConfig | null>(appConfig());
  const settings = useSettingsDraft(() => saved.value);
  const firstDraft = settings.draft.value;

  if (firstDraft === null) {
    throw new Error("The initial saved config must create a draft.");
  }

  expect(firstDraft).not.toBe(saved.value);
  firstDraft.osc.host = "vrchat.local";
  expect(saved.value?.osc.host).toBe("127.0.0.1");
  expect(settings.isDirty.value).toBe(true);

  saved.value = appConfig();
  await nextTick();

  expect(settings.draft.value).toBe(firstDraft);
  expect(settings.draft.value?.osc.host).toBe("vrchat.local");

  const changed = appConfig();
  changed.audio.inputDeviceId = "usb-headset";
  saved.value = changed;
  await nextTick();

  expect(settings.draft.value).not.toBe(firstDraft);
  expect(settings.draft.value).not.toBe(changed);
  expect(settings.draft.value?.audio.inputDeviceId).toBe("usb-headset");
  expect(settings.isDirty.value).toBe(false);
});

test("validates trimmed language hints without case-insensitive duplicates", () => {
  const saved = ref<AppConfig | null>(appConfig());
  const settings = useSettingsDraft(() => saved.value);

  if (settings.draft.value === null) {
    throw new Error("The initial saved config must create a draft.");
  }

  settings.draft.value.recognition.expectedLanguages = [" zh ", "EN"];
  expect(settings.hasValidExpectedLanguages.value).toBe(true);

  settings.draft.value.recognition.expectedLanguages = [" en ", "EN"];
  expect(settings.hasValidExpectedLanguages.value).toBe(false);
  expect(settings.createSaveConfig()).toBeNull();

  settings.draft.value.recognition.expectedLanguages = ["zh", "   "];
  expect(settings.hasValidExpectedLanguages.value).toBe(false);

  settings.draft.value.recognition.expectedLanguages = [];
  expect(settings.hasValidExpectedLanguages.value).toBe(false);
});

test("becomes clean again when edits return to the saved content", () => {
  const saved = ref<AppConfig | null>(appConfig());
  const settings = useSettingsDraft(() => saved.value);

  if (settings.draft.value === null) {
    throw new Error("The initial saved config must create a draft.");
  }

  settings.draft.value.ui.showOngoingPreview = false;
  expect(settings.isDirty.value).toBe(true);

  settings.draft.value.ui.showOngoingPreview = true;
  expect(settings.isDirty.value).toBe(false);
});

test("normalizes a detached save payload and retains the saved port fallback", () => {
  const saved = ref<AppConfig | null>(appConfig());
  const settings = useSettingsDraft(() => saved.value);

  if (settings.draft.value === null) {
    throw new Error("The initial saved config must create a draft.");
  }

  settings.draft.value.recognition.expectedLanguages = [" zh-CN ", "EN"];
  settings.draft.value.osc.host = "  vrchat.local  ";
  settings.draft.value.osc.port = Number.NaN;

  const next = settings.createSaveConfig();

  expect(next).toMatchObject({
    recognition: { expectedLanguages: ["zh-CN", "EN"] },
    translation: null,
    osc: { host: "vrchat.local", port: 9_000 },
    publication: { mode: "live", content: "sourceOnly" },
  });
  expect(next).not.toBe(settings.draft.value);
  expect(settings.draft.value.recognition.expectedLanguages).toEqual([
    " zh-CN ",
    "EN",
  ]);
  expect(settings.draft.value.osc.host).toBe("  vrchat.local  ");
  expect(settings.draft.value.osc.port).toBeNaN();

  if (next === null) {
    throw new Error("A valid draft must produce a save payload.");
  }
  next.audio.inputDeviceId = "changed-after-save";
  expect(settings.draft.value.audio.inputDeviceId).toBeNull();
});

test("preserves a finite edited port in the save payload", () => {
  const saved = ref<AppConfig | null>(appConfig());
  const settings = useSettingsDraft(() => saved.value);

  if (settings.draft.value === null) {
    throw new Error("The initial saved config must create a draft.");
  }

  settings.draft.value.osc.port = 9_001;

  expect(settings.createSaveConfig()?.osc.port).toBe(9_001);
});

test.each(["translationOnly", "bilingual"] as const)(
  "requires an explicit Translation target for %s content",
  (content) => {
    const saved = ref<AppConfig | null>(appConfig());
    const settings = useSettingsDraft(() => saved.value);

    settings.selectContent(content);

    expect(settings.draft.value?.publication.content).toBe(content);
    expect(settings.draft.value?.translation).toMatchObject({
      target: null,
      endpointKind: "official",
    });
    expect(settings.translationIssues.value).toEqual({
      target: "required",
      customApiBaseUrl: null,
    });
    expect(settings.canSave.value).toBe(false);
    expect(settings.createSaveConfig()).toBeNull();

    settings.selectTranslationTarget("zh-Hans");

    expect(settings.translationIssues.value).toEqual({
      target: null,
      customApiBaseUrl: null,
    });
    expect(settings.canSave.value).toBe(true);
    expect(settings.createSaveConfig()).toMatchObject({
      publication: { content },
      translation: {
        path: "openai/responses-completed-text",
        target: "zh-Hans",
        endpoint: { kind: "official" },
      },
    });
  },
);

test("supports both explicit targets without inferring from recognition hints", () => {
  const savedConfig = appConfig();
  savedConfig.recognition.expectedLanguages = ["zh-Hans"];
  const saved = ref<AppConfig | null>(savedConfig);
  const settings = useSettingsDraft(() => saved.value);

  settings.selectContent("translationOnly");
  expect(settings.draft.value?.translation?.target).toBeNull();

  settings.selectTranslationTarget("en");
  expect(settings.createSaveConfig()?.translation?.target).toBe("en");

  settings.selectTranslationTarget("zh-Hans");
  expect(settings.createSaveConfig()?.translation?.target).toBe("zh-Hans");
});

test("preserves dormant Translation while Source-only stays effective", () => {
  const savedConfig = appConfig();
  savedConfig.translation = {
    path: "openai/responses-completed-text",
    target: "en",
    endpoint: {
      kind: "custom",
      apiBaseUrl: "https://translation.example/v1",
    },
  };
  const saved = ref<AppConfig | null>(savedConfig);
  const settings = useSettingsDraft(() => saved.value);

  expect(settings.draft.value?.publication.content).toBe("sourceOnly");
  expect(settings.canSave.value).toBe(true);
  expect(settings.createSaveConfig()?.translation).toEqual(
    savedConfig.translation,
  );

  settings.selectContent("bilingual");
  settings.selectTranslationTarget("zh-Hans");
  settings.selectContent("sourceOnly");

  expect(settings.draft.value?.translation?.target).toBe("zh-Hans");
  expect(settings.createSaveConfig()).toMatchObject({
    publication: { content: "sourceOnly" },
    translation: {
      path: "openai/responses-completed-text",
      target: "zh-Hans",
      endpoint: {
        kind: "custom",
        apiBaseUrl: "https://translation.example/v1",
      },
    },
  });
});

test("preserves a dormant V2-compatible Custom URL but validates it when activated", () => {
  const apiBaseUrl = "https://example.com/api%/v1";
  const savedConfig = appConfig();
  savedConfig.translation = {
    path: "openai/responses-completed-text",
    target: "en",
    endpoint: { kind: "custom", apiBaseUrl },
  };
  const saved = ref<AppConfig | null>(savedConfig);
  const settings = useSettingsDraft(() => saved.value);

  expect(settings.isDirty.value).toBe(false);
  expect(settings.translationIssues.value.customApiBaseUrl).toBeNull();
  expect(settings.canSave.value).toBe(true);
  expect(settings.createSaveConfig()?.translation).toEqual(
    savedConfig.translation,
  );

  settings.selectContent("bilingual");

  expect(settings.translationIssues.value.customApiBaseUrl).toBe(
    "invalidPercentEncoding",
  );
  expect(settings.canSave.value).toBe(false);
  expect(settings.createSaveConfig()).toBeNull();
  expect(settings.draft.value?.translation?.customApiBaseUrl).toBe(apiBaseUrl);
});

test("retains an active V2-compatible Custom URL without inventing an edit", () => {
  const apiBaseUrl = "https://example.com/api%/v1";
  const savedConfig = appConfig();
  savedConfig.publication.content = "translationOnly";
  savedConfig.translation = {
    path: "openai/responses-completed-text",
    target: "en",
    endpoint: { kind: "custom", apiBaseUrl },
  };
  const saved = ref<AppConfig | null>(savedConfig);
  const settings = useSettingsDraft(() => saved.value);

  expect(settings.isDirty.value).toBe(false);
  expect(settings.translationIssues.value.customApiBaseUrl).toBe(
    "invalidPercentEncoding",
  );
  expect(settings.canSave.value).toBe(false);
  expect(settings.createSaveConfig()).toBeNull();
  expect(settings.draft.value?.translation?.customApiBaseUrl).toBe(apiBaseUrl);

  settings.setCustomTranslationApiBaseUrl("https://example.com/v1");

  expect(settings.isDirty.value).toBe(true);
  expect(settings.translationIssues.value.customApiBaseUrl).toBeNull();
  expect(settings.canSave.value).toBe(true);
});

test("does not replace a valid dormant Translation with an incomplete draft", () => {
  const savedConfig = appConfig();
  savedConfig.translation = {
    path: "openai/responses-completed-text",
    target: "en",
    endpoint: { kind: "official" },
  };
  const saved = ref<AppConfig | null>(savedConfig);
  const settings = useSettingsDraft(() => saved.value);

  settings.selectContent("translationOnly");
  const translation = settings.draft.value?.translation;
  if (!translation) {
    throw new Error("The saved Translation must create a Translation draft.");
  }
  translation.target = null;
  settings.selectContent("sourceOnly");

  expect(settings.canSave.value).toBe(true);
  expect(settings.createSaveConfig()?.translation).toEqual(
    savedConfig.translation,
  );
});

test("keeps Custom endpoint input raw and rejects it before save", () => {
  const saved = ref<AppConfig | null>(appConfig());
  const settings = useSettingsDraft(() => saved.value);

  settings.selectContent("bilingual");
  settings.selectTranslationTarget("en");
  settings.selectTranslationEndpoint("custom");
  settings.setCustomTranslationApiBaseUrl(" http://example.com/v1 ");

  expect(settings.draft.value?.translation?.customApiBaseUrl).toBe(
    " http://example.com/v1 ",
  );
  expect(settings.translationIssues.value.target).toBeNull();
  expect(settings.translationIssues.value.customApiBaseUrl).not.toBeNull();
  expect(settings.canSave.value).toBe(false);
  expect(settings.createSaveConfig()).toBeNull();

  settings.setCustomTranslationApiBaseUrl("https://example.com/v1");

  expect(settings.canSave.value).toBe(true);
  expect(settings.createSaveConfig()?.translation?.endpoint).toEqual({
    kind: "custom",
    apiBaseUrl: "https://example.com/v1",
  });
});

test("keeps an unsaved Custom URL while toggling the endpoint", () => {
  const saved = ref<AppConfig | null>(appConfig());
  const settings = useSettingsDraft(() => saved.value);

  settings.selectContent("translationOnly");
  settings.selectTranslationTarget("en");
  settings.selectTranslationEndpoint("custom");
  settings.setCustomTranslationApiBaseUrl("https://example.com/v1");
  settings.selectTranslationEndpoint("official");

  expect(settings.draft.value?.translation?.customApiBaseUrl).toBe(
    "https://example.com/v1",
  );
  expect(settings.createSaveConfig()?.translation?.endpoint).toEqual({
    kind: "official",
  });

  settings.selectTranslationEndpoint("custom");
  expect(settings.createSaveConfig()?.translation?.endpoint).toEqual({
    kind: "custom",
    apiBaseUrl: "https://example.com/v1",
  });
});

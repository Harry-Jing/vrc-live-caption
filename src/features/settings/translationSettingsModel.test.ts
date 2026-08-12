import { reactive } from "vue";
import { describe, expect, test } from "vitest";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
} from "../../runtime/appConfig";
import type {
  ContentSelection,
  TranslationTarget,
} from "../../runtime/captionPipeline";
import {
  createAppConfigFromTranslationSettings,
  createTranslationSettingsDraft,
  translationSettingsValidation,
  type TranslationSettingsDraft,
} from "./translationSettingsModel";

function appConfig(content: ContentSelection = "sourceOnly"): AppConfig {
  return {
    schemaVersion: APP_CONFIG_SCHEMA_VERSION,
    audio: { inputDeviceId: null },
    recognition: {
      path: "openai/gpt-live-transcribe",
      expectedLanguages: ["en", "zh"],
    },
    translation: null,
    osc: { host: "127.0.0.1", port: 9_000, enabled: true },
    publication: { mode: "completed", content },
    ui: { showOngoingPreview: true },
  };
}

function validDraft(
  overrides: Partial<TranslationSettingsDraft> = {},
): TranslationSettingsDraft {
  return {
    content: "bilingual",
    target: "zh-Hans",
    endpointKind: "official",
    customApiBaseUrl: "",
    ...overrides,
  };
}

describe("createTranslationSettingsDraft", () => {
  test.each<ContentSelection>(["sourceOnly", "translationOnly", "bilingual"])(
    "derives the %s content selection",
    (content) => {
      const config = appConfig(content);
      config.translation = {
        path: "openai/responses-completed-text",
        target: "en",
        endpoint: {
          kind: "custom",
          apiBaseUrl: "https://translation.example.test/v1",
        },
      };

      expect(createTranslationSettingsDraft(config)).toEqual({
        content,
        target: "en",
        endpointKind: "custom",
        customApiBaseUrl: "https://translation.example.test/v1",
      });
    },
  );

  test("uses an unselected target without inventing a translation choice", () => {
    expect(createTranslationSettingsDraft(appConfig())).toEqual({
      content: "sourceOnly",
      target: null,
      endpointKind: "official",
      customApiBaseUrl: "",
    });
  });
});

describe("translationSettingsValidation", () => {
  test.each<ContentSelection>(["translationOnly", "bilingual"])(
    "requires an explicit target for %s",
    (content) => {
      const validation = translationSettingsValidation(
        validDraft({ content, target: null }),
      );

      expect(validation.targetRequired).toBe(true);
      expect(validation.isValid).toBe(false);
    },
  );

  test("allows Source-only without a Translation selection", () => {
    expect(
      translationSettingsValidation(
        validDraft({ content: "sourceOnly", target: null }),
      ),
    ).toEqual({
      targetRequired: false,
      customApiBaseUrlError: null,
      isValid: true,
    });
  });

  test("does not silently discard a partial dormant Custom selection", () => {
    const validation = translationSettingsValidation(
      validDraft({
        content: "sourceOnly",
        target: null,
        endpointKind: "custom",
        customApiBaseUrl: "https://translation.example.test/v1",
      }),
    );

    expect(validation.targetRequired).toBe(true);
    expect(validation.isValid).toBe(false);
  });

  test.each([
    ["http://translation.example.test/v1", "httpsRequired"],
    ["https://user@example.test/v1", "userinfoForbidden"],
    ["https://translation.example.test/v1/responses", "responsesPathForbidden"],
  ])("rejects unsafe Custom base URL %s", (apiBaseUrl, message) => {
    const validation = translationSettingsValidation(
      validDraft({ endpointKind: "custom", customApiBaseUrl: apiBaseUrl }),
    );

    expect(validation.customApiBaseUrlError).toBe(message);
    expect(validation.isValid).toBe(false);
  });

  test("accepts a validated HTTPS Custom base URL", () => {
    expect(
      translationSettingsValidation(
        validDraft({
          endpointKind: "custom",
          customApiBaseUrl: "https://translation.example.test/v1",
        }),
      ),
    ).toEqual({
      targetRequired: false,
      customApiBaseUrlError: null,
      isValid: true,
    });
  });
});

describe("createAppConfigFromTranslationSettings", () => {
  for (const content of [
    "sourceOnly",
    "translationOnly",
    "bilingual",
  ] as const) {
    for (const target of ["en", "zh-Hans"] as const) {
      for (const endpointKind of ["official", "custom"] as const) {
        test(`supports ${content}, ${target}, and ${endpointKind}`, () => {
          const next = createAppConfigFromTranslationSettings(
            appConfig(),
            validDraft({
              content,
              target,
              endpointKind,
              customApiBaseUrl: "https://translation.example.test/v1",
            }),
          );

          expect(next).not.toBeNull();
          expect(next?.publication.content).toBe(content);
          expect(next?.translation?.target).toBe(target);
          expect(next?.translation?.endpoint.kind).toBe(endpointKind);
        });
      }
    }
  }

  test.each<TranslationTarget>(["en", "zh-Hans"])(
    "creates an Official selection with the explicit %s target",
    (target) => {
      const config = appConfig();
      const next = createAppConfigFromTranslationSettings(
        config,
        validDraft({ content: "translationOnly", target }),
      );

      expect(next).toMatchObject({
        translation: {
          path: "openai/responses-completed-text",
          target,
          endpoint: { kind: "official" },
        },
        publication: { content: "translationOnly" },
      });
      expect(next).not.toBe(config);
      expect(config.translation).toBeNull();
      expect(config.publication.content).toBe("sourceOnly");
    },
  );

  test("creates a validated Custom endpoint selection", () => {
    const next = createAppConfigFromTranslationSettings(
      appConfig(),
      validDraft({
        endpointKind: "custom",
        customApiBaseUrl: "https://translation.example.test/v1",
      }),
    );

    expect(next?.translation?.endpoint).toEqual({
      kind: "custom",
      apiBaseUrl: "https://translation.example.test/v1",
    });
  });

  test("trims a validated Custom endpoint before saving", () => {
    const next = createAppConfigFromTranslationSettings(
      appConfig(),
      validDraft({
        endpointKind: "custom",
        customApiBaseUrl: "  https://translation.example.test/v1  ",
      }),
    );

    expect(next?.translation?.endpoint).toEqual({
      kind: "custom",
      apiBaseUrl: "https://translation.example.test/v1",
    });
  });

  test("accepts a reactive settings form without mutating it", () => {
    const config = reactive(appConfig());

    const next = createAppConfigFromTranslationSettings(
      config,
      validDraft({ content: "translationOnly", target: "en" }),
    );

    expect(next?.publication.content).toBe("translationOnly");
    expect(config.publication.content).toBe("sourceOnly");
    expect(config.translation).toBeNull();
  });

  test("retains dormant Translation choices when Source-only is selected", () => {
    const config = appConfig("bilingual");
    config.translation = {
      path: "openai/responses-completed-text",
      target: "en",
      endpoint: {
        kind: "custom",
        apiBaseUrl: "https://translation.example.test/v1",
      },
    };
    const draft = createTranslationSettingsDraft(config);
    draft.content = "sourceOnly";

    const next = createAppConfigFromTranslationSettings(config, draft);

    expect(next?.publication.content).toBe("sourceOnly");
    expect(next?.translation).toEqual(config.translation);
  });

  test("keeps an entered Custom URL in the draft while Official is selected", () => {
    const draft = validDraft({
      endpointKind: "custom",
      customApiBaseUrl: "https://translation.example.test/v1",
    });

    draft.endpointKind = "official";
    const official = createAppConfigFromTranslationSettings(appConfig(), draft);
    draft.endpointKind = "custom";
    const custom = createAppConfigFromTranslationSettings(appConfig(), draft);

    expect(official?.translation?.endpoint).toEqual({ kind: "official" });
    expect(draft.customApiBaseUrl).toBe("https://translation.example.test/v1");
    expect(custom?.translation?.endpoint).toEqual({
      kind: "custom",
      apiBaseUrl: "https://translation.example.test/v1",
    });
  });

  test("keeps Source-only unconfigured when no Translation choice exists", () => {
    const next = createAppConfigFromTranslationSettings(
      appConfig(),
      createTranslationSettingsDraft(appConfig()),
    );

    expect(next?.translation).toBeNull();
    expect(next?.publication.content).toBe("sourceOnly");
  });

  test.each([
    validDraft({ content: "translationOnly", target: null }),
    validDraft({
      endpointKind: "custom",
      customApiBaseUrl: "http://translation.example.test/v1",
    }),
  ])("cannot create AppConfig from an invalid draft", (draft) => {
    expect(
      createAppConfigFromTranslationSettings(appConfig(), draft),
    ).toBeNull();
  });

  test("does not bypass validation elsewhere in AppConfig", () => {
    const invalidConfig = appConfig();
    invalidConfig.recognition.expectedLanguages = [];

    expect(
      createAppConfigFromTranslationSettings(invalidConfig, validDraft()),
    ).toBeNull();
  });
});

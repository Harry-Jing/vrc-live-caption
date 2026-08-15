import { describe, expect, test } from "vitest";
import {
  createPreviewAppGateway,
  previewCaptionPipelinePlan,
} from "./appGateway";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
} from "../../runtime/appConfig";
import type {
  PublicationMode,
  RecognitionPath,
} from "../../runtime/captionPipeline";

function config(path: RecognitionPath, mode: PublicationMode): AppConfig {
  return {
    schemaVersion: APP_CONFIG_SCHEMA_VERSION,
    audio: { inputDeviceId: null },
    recognition: { path, expectedLanguages: ["zh", "en"] },
    translation: null,
    osc: { host: "127.0.0.1", port: 9000, enabled: true },
    publication: { mode, content: "sourceOnly" },
    ui: { showOngoingPreview: true },
  };
}

describe("preview runtime planning", () => {
  test("keeps GPT Transcribe completed-only without rewriting Live", () => {
    const completed = previewCaptionPipelinePlan(
      config("openai/gpt-transcribe", "completed"),
    );
    const live = previewCaptionPipelinePlan(
      config("openai/gpt-transcribe", "live"),
    );

    expect(completed).toMatchObject({
      recognition: {
        path: "openai/gpt-transcribe",
        inputShape: "continuousAudioFrames",
        captionBoundaryOwner: "application",
        lanes: [{ updates: "completedOnly", revisions: "appendOnly" }],
      },
      publication: { state: "compatible", mode: "completed" },
    });
    expect(live.publication).toEqual({
      state: "incompatible",
      requestedMode: "live",
      selectedLanes: ["source"],
      reason: { reason: "modeUnsupported", lanes: ["source"] },
      supportedModes: ["completed"],
    });
  });

  test.each(["completed", "live"] as const)(
    "keeps GPT Live Transcribe compatible with %s publication",
    (mode) => {
      const plan = previewCaptionPipelinePlan(
        config("openai/gpt-live-transcribe", mode),
      );

      expect(plan.recognition).toMatchObject({
        path: "openai/gpt-live-transcribe",
        inputShape: "continuousAudioFrames",
        captionBoundaryOwner: "application",
        lanes: [
          {
            updates: "ongoingAndCompleted",
            revisions: "revisableFullSnapshot",
          },
        ],
      });
      expect(plan.publication).toMatchObject({ state: "compatible", mode });
    },
  );

  test.each([
    ["translationOnly", "completed", "compatible", ["translation"]],
    ["translationOnly", "live", "incompatible", ["translation"]],
    ["bilingual", "completed", "compatible", ["source", "translation"]],
    ["bilingual", "live", "incompatible", ["source", "translation"]],
  ] as const)(
    "plans %s content with %s publication without rewriting it",
    (content, mode, state, selectedLanes) => {
      const request = config("openai/gpt-live-transcribe", mode);
      request.translation = {
        path: "openai/responses-completed-text",
        target: "zh-Hans",
        endpoint: { kind: "official" },
      };
      request.publication.content = content;

      const plan = previewCaptionPipelinePlan(request);

      expect(plan.translation).toMatchObject({
        path: "openai/responses-completed-text",
        inputShape: "completedSourceSnapshots",
      });
      expect(plan.publication).toMatchObject({ state, selectedLanes });
      if (mode === "live") {
        expect(plan.publication).toMatchObject({
          reason: { reason: "modeUnsupported", lanes: ["translation"] },
          supportedModes: ["completed"],
        });
      }
      expect(request.publication).toEqual({ mode, content });
    },
  );

  test("keeps a saved Translation selection dormant for Source-only Live", () => {
    const request = config("openai/gpt-live-transcribe", "live");
    request.translation = {
      path: "openai/responses-completed-text",
      target: "zh-Hans",
      endpoint: {
        kind: "custom",
        apiBaseUrl: "https://example.com/v1",
      },
    };

    expect(previewCaptionPipelinePlan(request)).toMatchObject({
      translation: null,
      publication: {
        state: "compatible",
        selectedLanes: ["source"],
      },
    });
  });
});

describe("preview App Config V2 validation", () => {
  test("round-trips every supported content and endpoint shape", async () => {
    const gateway = createPreviewAppGateway();
    const sourceOnlyWithDormantCustom = config(
      "openai/gpt-transcribe",
      "completed",
    );
    sourceOnlyWithDormantCustom.translation = {
      path: "openai/responses-completed-text",
      target: "zh-Hans",
      endpoint: {
        kind: "custom",
        apiBaseUrl: "https://translation.example/v1",
      },
    };

    const translationOnlyOfficial = config(
      "openai/gpt-transcribe",
      "completed",
    );
    translationOnlyOfficial.publication.content = "translationOnly";
    translationOnlyOfficial.translation = {
      path: "openai/responses-completed-text",
      target: "en",
      endpoint: { kind: "official" },
    };

    const bilingualCustom = config("openai/gpt-live-transcribe", "completed");
    bilingualCustom.publication.content = "bilingual";
    bilingualCustom.translation = {
      path: "openai/responses-completed-text",
      target: "zh-Hans",
      endpoint: {
        kind: "custom",
        apiBaseUrl: "https://translation.example/v1",
      },
    };

    for (const expected of [
      sourceOnlyWithDormantCustom,
      translationOnlyOfficial,
      bilingualCustom,
    ]) {
      const saved = await gateway.saveAppConfig(expected);
      const pulled = await gateway.getRuntimeControlSnapshot();

      expect(saved.desired.config).toEqual(expected);
      expect(pulled.desired.config).toEqual(expected);
      expect(pulled.generation).toBeNull();
    }
  });

  test("matches App Config V2 compatibility for an existing Custom URL", async () => {
    const gateway = createPreviewAppGateway();
    const expected = config("openai/gpt-transcribe", "completed");
    expected.publication.content = "bilingual";
    expected.translation = {
      path: "openai/responses-completed-text",
      target: "zh-Hans",
      endpoint: {
        kind: "custom",
        apiBaseUrl: "https://example.com/api%/v1",
      },
    };

    const saved = await gateway.saveAppConfig(expected);

    expect(saved.desired.config).toEqual(expected);
    expect((await gateway.getRuntimeControlSnapshot()).desired.config).toEqual(
      expected,
    );
  });

  test("rejects Translation content without a selection and retains desired settings", async () => {
    const gateway = createPreviewAppGateway();
    const before = await gateway.getRuntimeControlSnapshot();
    const invalid = config("openai/gpt-transcribe", "completed");
    invalid.publication.content = "translationOnly";

    await expect(gateway.saveAppConfig(invalid)).rejects.toThrow(
      /Translation content requires a translation selection/u,
    );

    expect((await gateway.getRuntimeControlSnapshot()).desired.config).toEqual(
      before.desired.config,
    );
  });

  test.each([
    "http://example.com/v1",
    "https://user:secret@example.com/v1",
    "https://example.com/v1?",
    "https://example.com/v1/%72esponses",
  ])("rejects an invalid Custom API base URL %s", async (apiBaseUrl) => {
    const gateway = createPreviewAppGateway();
    const invalid = config("openai/gpt-transcribe", "completed");
    invalid.translation = {
      path: "openai/responses-completed-text",
      target: "zh-Hans",
      endpoint: { kind: "custom", apiBaseUrl },
    };

    await expect(gateway.saveAppConfig(invalid)).rejects.toThrow(
      /API base URL/u,
    );
  });
});

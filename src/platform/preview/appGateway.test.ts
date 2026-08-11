import { describe, expect, test } from "vitest";
import { previewCaptionPipelinePlan } from "./appGateway";
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
    osc: { host: "127.0.0.1", port: 9000, enabled: true },
    publication: { mode },
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
});

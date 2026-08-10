import { describe, expect, test } from "vitest";
import { previewRuntimePlan } from "./runtimeBackend";
import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
  type OpenAiTranscriptionModel,
  type PublicationMode,
} from "../runtime/types";

function config(
  model: OpenAiTranscriptionModel,
  mode: PublicationMode,
): AppConfig {
  return {
    schemaVersion: APP_CONFIG_SCHEMA_VERSION,
    audio: { inputDeviceId: null },
    stt: { provider: "openai", languages: ["zh", "en"], model },
    osc: { host: "127.0.0.1", port: 9000, enabled: true },
    publication: { mode },
    ui: { showPartial: true },
  };
}

describe("preview runtime planning", () => {
  test("keeps GPT Transcribe completed-only without rewriting Live", () => {
    const completed = previewRuntimePlan(config("gpt-transcribe", "completed"));
    const live = previewRuntimePlan(config("gpt-transcribe", "live"));

    expect(completed).toMatchObject({
      recognition: {
        path: "openAiGptTranscribe",
        inputShape: "continuousAudioFrames",
        boundaryOwner: "application",
        lanes: [{ updates: "completedOnly", revisions: "appendOnly" }],
      },
      publication: { state: "ready", mode: "completed" },
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
    "keeps GPT Live Transcribe ready with %s publication",
    (mode) => {
      const plan = previewRuntimePlan(config("gpt-live-transcribe", mode));

      expect(plan.recognition).toMatchObject({
        path: "openAiGptLiveTranscribe",
        inputShape: "continuousAudioFrames",
        boundaryOwner: "application",
        lanes: [
          {
            updates: "ongoingAndCompleted",
            revisions: "revisableFullSnapshot",
          },
        ],
      });
      expect(plan.publication).toMatchObject({ state: "ready", mode });
    },
  );
});

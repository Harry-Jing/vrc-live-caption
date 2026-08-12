import {
  APP_CONFIG_SCHEMA_VERSION,
  type AppConfig,
} from "../../runtime/appConfig";
import {
  CAPTION_AGGREGATE_CONTRACT_VERSION,
  type CaptionAggregateSnapshot,
  type CaptionSnapshot,
  type SourceSnapshotRef,
  type TranslationFailureReason,
} from "../../runtime/captionAggregate";
import type { RuntimeGenerationSnapshot } from "../../runtime/runtimeControl";
import type { RuntimeStatusEvent } from "../../runtime/runtimeEvents";

export const PREVIEW_TRANSLATION_SCENARIOS = [
  "official-pending",
  "official-success",
  "official-failed",
  "official-degraded",
  "custom-success",
  "stopped",
  "restarted",
] as const;

export type PreviewTranslationScenario =
  (typeof PREVIEW_TRANSLATION_SCENARIOS)[number];

export type PreviewTranslationScenarioSeed = Readonly<{
  config: AppConfig;
  generation: Omit<RuntimeGenerationSnapshot, "captionPipelinePlan"> | null;
  captionAggregate: CaptionAggregateSnapshot;
  runtimeStatus: RuntimeStatusEvent;
  credentialSuffixes: Readonly<{
    openai: string | null;
    customTranslation: string | null;
  }>;
}>;

export function previewTranslationScenarioFromSearch(
  search: string,
): PreviewTranslationScenario | null {
  const requested = new URLSearchParams(search).get("translationScenario");

  return (
    PREVIEW_TRANSLATION_SCENARIOS.find((scenario) => scenario === requested) ??
    null
  );
}

export function createPreviewTranslationScenarioSeed(
  scenario: PreviewTranslationScenario,
): PreviewTranslationScenarioSeed {
  if (scenario === "stopped") {
    const config = translationConfig("translationOnly", "official");

    return {
      config,
      generation: null,
      captionAggregate: aggregate(null, completedPair(7, "official-stopped")),
      runtimeStatus: status("stopped", "Preview Translation scenario stopped"),
      credentialSuffixes: { openai: "1234", customTranslation: null },
    };
  }

  const generationId = scenario === "restarted" ? 8 : 7;
  const endpointKind = scenario === "custom-success" ? "custom" : "official";
  const content =
    scenario === "official-failed" ? "translationOnly" : "bilingual";
  const config = translationConfig(content, endpointKind);
  const streamId = stream(generationId);
  const scenarioState = scenarioStateFor(scenario, generationId);
  const captions =
    scenario === "restarted"
      ? [
          ...scenarioState.captions,
          ...completedPair(7, "stale-prior-generation").captions,
        ]
      : scenarioState.captions;
  const translationUnits =
    scenario === "restarted"
      ? [
          ...scenarioState.translationUnits,
          ...completedPair(7, "stale-prior-generation").translationUnits,
        ]
      : scenarioState.translationUnits;

  return {
    config,
    generation: {
      id: generationId,
      phase: "running",
      startedFromConfigRevision: 1,
      selection: {
        audio: { ...config.audio },
        recognition: {
          ...config.recognition,
          expectedLanguages: [...config.recognition.expectedLanguages],
        },
        translation: config.translation,
        osc: { ...config.osc },
        publication: { ...config.publication },
      },
      credentials: [
        {
          id: "openai",
          storage: "systemCredentialStore",
          displaySuffix: "1234",
          revision: 1,
        },
        ...(endpointKind === "custom"
          ? [
              {
                id: "customTranslation" as const,
                storage: "systemCredentialStore" as const,
                displaySuffix: "5678",
                revision: 1,
              },
            ]
          : []),
      ],
      chatboxPublication: {
        state: "ready",
        host: config.osc.host,
        port: config.osc.port,
      },
      translationState:
        scenario === "official-degraded"
          ? {
              state: "degraded",
              reasonCode: "translation.provider_unavailable",
            }
          : { state: "active" },
      uploadsMicrophoneAudio: true,
      uploadsSourceText: true,
    },
    captionAggregate: aggregate(
      { generation: generationId, streamId },
      { captions, translationUnits },
    ),
    runtimeStatus: status("running", "Preview Translation scenario running"),
    credentialSuffixes: {
      openai: "1234",
      customTranslation: endpointKind === "custom" ? "5678" : null,
    },
  };
}

function translationConfig(
  content: "translationOnly" | "bilingual",
  endpointKind: "official" | "custom",
): AppConfig {
  return {
    schemaVersion: APP_CONFIG_SCHEMA_VERSION,
    audio: { inputDeviceId: null },
    recognition: {
      path: "openai/gpt-transcribe",
      expectedLanguages: ["en"],
    },
    translation: {
      path: "openai/responses-completed-text",
      target: "zh-Hans",
      endpoint:
        endpointKind === "official"
          ? { kind: "official" }
          : {
              kind: "custom",
              apiBaseUrl: "https://translation.example.test/v1",
            },
    },
    osc: { host: "127.0.0.1", port: 9000, enabled: true },
    publication: { mode: "completed", content },
    ui: { showOngoingPreview: true },
  };
}

function scenarioStateFor(
  scenario: Exclude<PreviewTranslationScenario, "stopped">,
  generation: number,
) {
  switch (scenario) {
    case "official-pending":
    case "restarted":
      return pendingUnit(generation, `${scenario}-unit`);
    case "official-success":
    case "custom-success":
      return completedPair(generation, `${scenario}-unit`);
    case "official-failed":
      return failedUnit(
        generation,
        "official-failed-unit",
        "translation.provider_rate_limited",
      );
    case "official-degraded": {
      const failed = failedUnit(
        generation,
        "degraded-failed-unit",
        "translation.provider_unavailable",
      );
      const pending = pendingUnit(generation, "degraded-pending-unit");
      const completed = completedPair(generation, "degraded-completed-unit");

      return {
        captions: [
          ...failed.captions,
          ...pending.captions,
          ...completed.captions,
        ],
        translationUnits: [
          ...failed.translationUnits,
          ...pending.translationUnits,
          ...completed.translationUnits,
        ],
      };
    }
  }
}

function pendingUnit(generation: number, unitId: string) {
  const source = sourceCaption(generation, unitId);

  return {
    captions: [source],
    translationUnits: [{ state: "pending" as const, sourceRef: ref(source) }],
  };
}

function completedPair(generation: number, unitId: string) {
  const source = sourceCaption(generation, unitId);
  const sourceRef = ref(source);

  return {
    captions: [
      source,
      {
        ...source,
        lane: "translation" as const,
        text: `第 ${String(generation)} 代的确定性译文。`,
        language: "zh-Hans",
        sourceRef,
        timestampMs: source.timestampMs + 1,
      },
    ],
    translationUnits: [{ state: "completed" as const, sourceRef }],
  };
}

function failedUnit(
  generation: number,
  unitId: string,
  reasonCode: TranslationFailureReason,
) {
  const source = sourceCaption(generation, unitId);

  return {
    captions: [source],
    translationUnits: [
      { state: "failed" as const, sourceRef: ref(source), reasonCode },
    ],
  };
}

function sourceCaption(generation: number, unitId: string): CaptionSnapshot {
  return {
    generation,
    streamId: stream(generation),
    unitId,
    lane: "source",
    revision: 1,
    text: `Deterministic Source for ${unitId}.`,
    state: "completed",
    language: "en",
    sourceRef: null,
    unitStartedAtMs: generation * 1_000,
    timestampMs: generation * 1_000 + 500,
  };
}

function ref(source: CaptionSnapshot): SourceSnapshotRef {
  if (source.unitId === null) {
    throw new Error("Preview completed Source must be unit-scoped.");
  }

  return {
    generation: source.generation,
    streamId: source.streamId,
    unitId: source.unitId,
    revision: source.revision,
  };
}

function aggregate(
  activeStream: CaptionAggregateSnapshot["activeStream"],
  content: Readonly<{
    captions: readonly CaptionSnapshot[];
    translationUnits: CaptionAggregateSnapshot["translationUnits"];
  }>,
): CaptionAggregateSnapshot {
  return {
    contractVersion: CAPTION_AGGREGATE_CONTRACT_VERSION,
    snapshotRevision: 9,
    activeStream,
    openSourceUnits: [],
    captions: content.captions,
    translationUnits: content.translationUnits,
  };
}

function stream(generation: number) {
  return `recognition-${String(generation)}-1`;
}

function status(
  status: RuntimeStatusEvent["status"],
  message: string,
): RuntimeStatusEvent {
  return { status, message, timestampMs: 1_700_000_000_000 };
}

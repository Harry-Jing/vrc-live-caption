// Decodes Rust-owned Runtime Control payloads at the Tauri IPC boundary. Exact
// field sets and cross-field checks turn contract drift into an explicit failure.
import { createDecoders } from "./contractDecoding";
import {
  APP_CONFIG_SCHEMA_VERSION,
  translationApiBaseUrlValidationError,
  type AppConfig,
  type TranslationConfig,
  type TranslationEndpoint,
} from "../appConfig";
import { CAPTION_LANES, type CaptionLane } from "../captionAggregate";
import {
  CAPTION_BOUNDARY_OWNERS,
  CAPTION_UNIT_BEHAVIORS,
  CONTENT_SELECTIONS,
  LANE_UPDATE_BEHAVIORS,
  PUBLICATION_INCOMPATIBILITY_REASONS,
  PUBLICATION_MODES,
  PUBLICATION_PLAN_STATES,
  RECOGNITION_INPUT_SHAPES,
  RECOGNITION_PATHS,
  RESOLVED_PUBLICATION_TIMINGS,
  REVISION_BEHAVIORS,
  TRANSLATION_ENDPOINT_KINDS,
  TRANSLATION_INPUT_SHAPES,
  TRANSLATION_PATHS,
  TRANSLATION_TARGETS,
  type CaptionBoundaryOwner,
  type CaptionPipelinePlan,
  type CaptionUnitBehavior,
  type ContentSelection,
  type LaneUpdateBehavior,
  type PublicationMode,
  type PublicationPlan,
  type RecognitionCapabilityProfile,
  type RecognitionInputShape,
  type RecognitionPath,
  type ResolvedPublicationTiming,
  type RevisionBehavior,
  type TranslationCapabilityProfile,
  type TranslationInputShape,
  type TranslationPath,
  type TranslationTarget,
} from "../captionPipeline";
import {
  CHATBOX_PUBLICATION_STATES,
  CREDENTIAL_IDS,
  CREDENTIAL_STATUS_STATES,
  CREDENTIAL_STORAGES,
  RUNTIME_GENERATION_PHASES,
  RUNTIME_PENDING_GENERATION_CHANGES,
  RUNTIME_CONTROL_CONTRACT_VERSION,
  type ChatboxPublicationSnapshot,
  type CredentialId,
  type CredentialStatus,
  type CredentialStorage,
  type RuntimeControlSnapshot,
  type RuntimeGenerationCredentialSnapshot,
  type RuntimeGenerationPhase,
  type RuntimeGenerationSnapshot,
  type RuntimePendingGenerationChange,
} from "../runtimeControl";
import {
  RUNTIME_STATUSES,
  type RuntimeStatus,
  type RuntimeStatusEvent,
} from "../runtimeEvents";
export class RuntimeControlContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`Invalid runtime control payload at ${path}: ${expectation}.`);
    this.name = "RuntimeControlContractError";
  }
}

const { record, exactRecord, array, string, boolean, safeInteger, literal } =
  createDecoders(RuntimeControlContractError);

function nullableString(value: unknown, path: string): string | null {
  return value === null ? null : string(value, path);
}

function languageHints(value: unknown, path: string): string[] {
  const hints = array(value, path).map((hint, index) => {
    const decoded = string(hint, `${path}[${String(index)}]`);

    if (decoded.trim().length === 0) {
      throw new RuntimeControlContractError(
        `${path}[${String(index)}]`,
        "expected a non-empty language hint",
      );
    }

    return decoded;
  });

  if (hints.length === 0) {
    throw new RuntimeControlContractError(
      path,
      "expected at least one language hint",
    );
  }

  const normalized = hints.map((hint) => hint.trim().toLocaleLowerCase("en"));
  if (new Set(normalized).size !== normalized.length) {
    throw new RuntimeControlContractError(
      path,
      "expected unique language hints",
    );
  }

  return hints;
}

function decodeAudioConfig(value: unknown, path: string): AppConfig["audio"] {
  const input = exactRecord(value, path, ["inputDeviceId"]);

  return {
    inputDeviceId: nullableString(
      input["inputDeviceId"],
      `${path}.inputDeviceId`,
    ),
  };
}

function decodeRecognitionConfig(
  value: unknown,
  path: string,
): AppConfig["recognition"] {
  const input = exactRecord(value, path, ["path", "expectedLanguages"]);

  return {
    path: literal<RecognitionPath>(
      input["path"],
      `${path}.path`,
      RECOGNITION_PATHS,
    ),
    expectedLanguages: languageHints(
      input["expectedLanguages"],
      `${path}.expectedLanguages`,
    ),
  };
}

function decodeApiBaseUrl(value: unknown, path: string): string {
  const raw = string(value, path);
  const validationError = translationApiBaseUrlValidationError(raw);
  if (validationError !== null) {
    throw new RuntimeControlContractError(path, validationError);
  }

  return raw;
}

function decodeTranslationEndpoint(
  value: unknown,
  path: string,
): TranslationEndpoint {
  const tagged = record(value, path);
  const kind = literal(
    tagged["kind"],
    `${path}.kind`,
    TRANSLATION_ENDPOINT_KINDS,
  );

  switch (kind) {
    case "official":
      exactRecord(value, path, ["kind"]);
      return { kind };
    case "custom": {
      const input = exactRecord(value, path, ["kind", "apiBaseUrl"]);
      return {
        kind,
        apiBaseUrl: decodeApiBaseUrl(input["apiBaseUrl"], `${path}.apiBaseUrl`),
      };
    }
  }
}

function decodeTranslationConfig(
  value: unknown,
  path: string,
): TranslationConfig {
  const input = exactRecord(value, path, ["path", "target", "endpoint"]);

  return {
    path: literal<TranslationPath>(
      input["path"],
      `${path}.path`,
      TRANSLATION_PATHS,
    ),
    target: literal<TranslationTarget>(
      input["target"],
      `${path}.target`,
      TRANSLATION_TARGETS,
    ),
    endpoint: decodeTranslationEndpoint(input["endpoint"], `${path}.endpoint`),
  };
}

function decodeOscConfig(value: unknown, path: string): AppConfig["osc"] {
  const input = exactRecord(value, path, ["host", "port", "enabled"]);

  return {
    host: string(input["host"], `${path}.host`),
    port: safeInteger(input["port"], `${path}.port`, 0, 65_535),
    enabled: boolean(input["enabled"], `${path}.enabled`),
  };
}

function decodePublicationConfig(
  value: unknown,
  path: string,
): AppConfig["publication"] {
  const input = exactRecord(value, path, ["mode", "content"]);

  return {
    mode: literal<PublicationMode>(
      input["mode"],
      `${path}.mode`,
      PUBLICATION_MODES,
    ),
    content: literal<ContentSelection>(
      input["content"],
      `${path}.content`,
      CONTENT_SELECTIONS,
    ),
  };
}

function decodeAppConfig(value: unknown, path: string): AppConfig {
  const input = exactRecord(value, path, [
    "schemaVersion",
    "audio",
    "recognition",
    "translation",
    "osc",
    "publication",
    "ui",
  ]);
  if (input["schemaVersion"] !== APP_CONFIG_SCHEMA_VERSION) {
    throw new RuntimeControlContractError(
      `${path}.schemaVersion`,
      `expected ${String(APP_CONFIG_SCHEMA_VERSION)}`,
    );
  }
  const ui = exactRecord(input["ui"], `${path}.ui`, ["showOngoingPreview"]);
  const publication = decodePublicationConfig(
    input["publication"],
    `${path}.publication`,
  );
  const translation =
    input["translation"] === null
      ? null
      : decodeTranslationConfig(input["translation"], `${path}.translation`);
  if (publication.content !== "sourceOnly" && translation === null) {
    throw new RuntimeControlContractError(
      `${path}.translation`,
      "translation content requires a translation selection",
    );
  }

  return {
    schemaVersion: APP_CONFIG_SCHEMA_VERSION,
    audio: decodeAudioConfig(input["audio"], `${path}.audio`),
    recognition: decodeRecognitionConfig(
      input["recognition"],
      `${path}.recognition`,
    ),
    translation,
    osc: decodeOscConfig(input["osc"], `${path}.osc`),
    publication,
    ui: {
      showOngoingPreview: boolean(
        ui["showOngoingPreview"],
        `${path}.ui.showOngoingPreview`,
      ),
    },
  };
}

function decodeCaptionLanes(value: unknown, path: string): CaptionLane[] {
  return array(value, path).map((lane, index) =>
    literal<CaptionLane>(lane, `${path}[${String(index)}]`, CAPTION_LANES),
  );
}

function decodeLaneCapability(
  value: unknown,
  path: string,
): RecognitionCapabilityProfile["lanes"][number] {
  const input = exactRecord(value, path, ["lane", "updates", "revisions"]);

  return {
    lane: literal<CaptionLane>(input["lane"], `${path}.lane`, CAPTION_LANES),
    updates: literal<LaneUpdateBehavior>(
      input["updates"],
      `${path}.updates`,
      LANE_UPDATE_BEHAVIORS,
    ),
    revisions: literal<RevisionBehavior>(
      input["revisions"],
      `${path}.revisions`,
      REVISION_BEHAVIORS,
    ),
  };
}

function decodeRecognitionProfile(
  value: unknown,
  path: string,
): RecognitionCapabilityProfile {
  const input = exactRecord(value, path, [
    "path",
    "inputShape",
    "captionBoundaryOwner",
    "unitBehavior",
    "lanes",
  ]);

  return {
    path: literal<RecognitionPath>(
      input["path"],
      `${path}.path`,
      RECOGNITION_PATHS,
    ),
    inputShape: literal<RecognitionInputShape>(
      input["inputShape"],
      `${path}.inputShape`,
      RECOGNITION_INPUT_SHAPES,
    ),
    captionBoundaryOwner: literal<CaptionBoundaryOwner>(
      input["captionBoundaryOwner"],
      `${path}.captionBoundaryOwner`,
      CAPTION_BOUNDARY_OWNERS,
    ),
    unitBehavior: literal<CaptionUnitBehavior>(
      input["unitBehavior"],
      `${path}.unitBehavior`,
      CAPTION_UNIT_BEHAVIORS,
    ),
    lanes: array(input["lanes"], `${path}.lanes`).map((lane, index) =>
      decodeLaneCapability(lane, `${path}.lanes[${String(index)}]`),
    ),
  };
}

function decodeTranslationProfile(
  value: unknown,
  path: string,
): TranslationCapabilityProfile {
  const input = exactRecord(value, path, ["path", "inputShape", "lanes"]);
  const lanes = array(input["lanes"], `${path}.lanes`).map((lane, index) =>
    decodeLaneCapability(lane, `${path}.lanes[${String(index)}]`),
  );

  return {
    path: literal<TranslationPath>(
      input["path"],
      `${path}.path`,
      TRANSLATION_PATHS,
    ),
    inputShape: literal<TranslationInputShape>(
      input["inputShape"],
      `${path}.inputShape`,
      TRANSLATION_INPUT_SHAPES,
    ),
    lanes,
  };
}

function decodeResolvedTiming(
  value: unknown,
  path: string,
): ResolvedPublicationTiming {
  const tagged = record(value, path);
  const timing = literal(
    tagged["timing"],
    `${path}.timing`,
    RESOLVED_PUBLICATION_TIMINGS,
  );

  switch (timing) {
    case "completed": {
      exactRecord(value, path, ["timing"]);
      return { timing };
    }
    case "liveUnit": {
      const input = exactRecord(value, path, ["timing", "observationWindowMs"]);
      return {
        timing,
        observationWindowMs: safeInteger(
          input["observationWindowMs"],
          `${path}.observationWindowMs`,
          1,
        ),
      };
    }
  }
}

function assertPlanMatchesMode(
  planMode: PublicationMode,
  expectedMode: PublicationMode,
  path: string,
) {
  if (planMode !== expectedMode) {
    throw new RuntimeControlContractError(
      path,
      `expected the configured mode ${expectedMode}`,
    );
  }
}

function decodePublicationPlan(
  value: unknown,
  path: string,
  expectedMode: PublicationMode,
): PublicationPlan {
  const tagged = record(value, path);
  const state = literal(
    tagged["state"],
    `${path}.state`,
    PUBLICATION_PLAN_STATES,
  );

  switch (state) {
    case "compatible": {
      const input = exactRecord(value, path, [
        "state",
        "mode",
        "timing",
        "selectedLanes",
      ]);
      const mode = literal<PublicationMode>(
        input["mode"],
        `${path}.mode`,
        PUBLICATION_MODES,
      );
      const timing = decodeResolvedTiming(input["timing"], `${path}.timing`);
      assertPlanMatchesMode(mode, expectedMode, `${path}.mode`);
      if (
        (mode === "completed" && timing.timing !== "completed") ||
        (mode === "live" && timing.timing === "completed")
      ) {
        throw new RuntimeControlContractError(
          `${path}.timing`,
          `timing does not implement ${mode} mode`,
        );
      }

      return {
        state,
        mode,
        timing,
        selectedLanes: decodeCaptionLanes(
          input["selectedLanes"],
          `${path}.selectedLanes`,
        ),
      };
    }
    case "incompatible": {
      const input = exactRecord(value, path, [
        "state",
        "requestedMode",
        "selectedLanes",
        "reason",
        "supportedModes",
      ]);
      const requestedMode = literal<PublicationMode>(
        input["requestedMode"],
        `${path}.requestedMode`,
        PUBLICATION_MODES,
      );
      assertPlanMatchesMode(
        requestedMode,
        expectedMode,
        `${path}.requestedMode`,
      );
      const reasonPath = `${path}.reason`;
      const taggedReason = record(input["reason"], reasonPath);
      const reason = literal(
        taggedReason["reason"],
        `${reasonPath}.reason`,
        PUBLICATION_INCOMPATIBILITY_REASONS,
      );
      const decodedReason = (() => {
        switch (reason) {
          case "noLanesSelected": {
            exactRecord(input["reason"], reasonPath, ["reason"]);
            return { reason } as const;
          }
          case "laneUnavailable":
          case "modeUnsupported": {
            const decoded = exactRecord(input["reason"], reasonPath, [
              "reason",
              "lanes",
            ]);
            return {
              reason,
              lanes: decodeCaptionLanes(
                decoded["lanes"],
                `${reasonPath}.lanes`,
              ),
            } as const;
          }
        }
      })();

      return {
        state,
        requestedMode,
        selectedLanes: decodeCaptionLanes(
          input["selectedLanes"],
          `${path}.selectedLanes`,
        ),
        reason: decodedReason,
        supportedModes: array(
          input["supportedModes"],
          `${path}.supportedModes`,
        ).map((mode, index) =>
          literal<PublicationMode>(
            mode,
            `${path}.supportedModes[${String(index)}]`,
            PUBLICATION_MODES,
          ),
        ),
      };
    }
  }
}

function decodeCaptionPipelinePlan(
  value: unknown,
  path: string,
  expectedPublication: AppConfig["publication"],
  expectedTranslation: TranslationConfig | null,
): CaptionPipelinePlan {
  const input = exactRecord(value, path, [
    "recognition",
    "translation",
    "publication",
  ]);

  const translation =
    input["translation"] === null
      ? null
      : decodeTranslationProfile(input["translation"], `${path}.translation`);
  const translationIsActive = expectedPublication.content !== "sourceOnly";
  if (translationIsActive !== (translation !== null)) {
    throw new RuntimeControlContractError(
      `${path}.translation`,
      translationIsActive
        ? "translation content requires an active Translation profile"
        : "Source-only content requires Translation to remain dormant",
    );
  }
  if (translation !== null && expectedTranslation === null) {
    throw new RuntimeControlContractError(
      `${path}.translation.path`,
      "expected the selected Translation path",
    );
  }
  if (translation !== null) {
    const [translationLane] = translation.lanes;
    if (
      translationLane === undefined ||
      translation.lanes.length !== 1 ||
      translationLane.lane !== "translation" ||
      translationLane.updates !== "completedOnly" ||
      translationLane.revisions !== "appendOnly"
    ) {
      throw new RuntimeControlContractError(
        `${path}.translation.lanes`,
        "expected the completed Translation capability profile",
      );
    }
  }
  const publication = decodePublicationPlan(
    input["publication"],
    `${path}.publication`,
    expectedPublication.mode,
  );
  const expectedLanes: readonly CaptionLane[] =
    expectedPublication.content === "sourceOnly"
      ? ["source"]
      : expectedPublication.content === "translationOnly"
        ? ["translation"]
        : ["source", "translation"];
  if (
    publication.selectedLanes.length !== expectedLanes.length ||
    publication.selectedLanes.some(
      (lane, index) => lane !== expectedLanes[index],
    )
  ) {
    throw new RuntimeControlContractError(
      `${path}.publication.selectedLanes`,
      `expected lanes selected by ${expectedPublication.content}`,
    );
  }

  return {
    recognition: decodeRecognitionProfile(
      input["recognition"],
      `${path}.recognition`,
    ),
    translation,
    publication,
  };
}

function decodeCredentialStatus(
  value: unknown,
  path: string,
): CredentialStatus {
  const tagged = record(value, path);
  const state = literal(
    tagged["state"],
    `${path}.state`,
    CREDENTIAL_STATUS_STATES,
  );

  switch (state) {
    case "unconfigured": {
      const input = exactRecord(value, path, ["state", "id"]);
      return {
        state,
        id: literal<CredentialId>(input["id"], `${path}.id`, CREDENTIAL_IDS),
      };
    }
    case "configured": {
      const input = exactRecord(value, path, [
        "state",
        "id",
        "storage",
        "displaySuffix",
      ]);
      const id = literal<CredentialId>(
        input["id"],
        `${path}.id`,
        CREDENTIAL_IDS,
      );
      const storage = literal<CredentialStorage>(
        input["storage"],
        `${path}.storage`,
        CREDENTIAL_STORAGES,
      );
      assertCredentialStorage(id, storage, `${path}.storage`);

      return {
        state,
        id,
        storage,
        displaySuffix: nullableString(
          input["displaySuffix"],
          `${path}.displaySuffix`,
        ),
      };
    }
    case "unavailable": {
      const input = exactRecord(value, path, ["state", "id", "failure"]);
      const failure = exactRecord(input["failure"], `${path}.failure`, [
        "code",
        "message",
      ]);
      return {
        state,
        id: literal<CredentialId>(input["id"], `${path}.id`, CREDENTIAL_IDS),
        failure: {
          code: string(failure["code"], `${path}.failure.code`),
          message: string(failure["message"], `${path}.failure.message`),
        },
      };
    }
  }
}

function assertCredentialStorage(
  id: CredentialId,
  storage: CredentialStorage,
  path: string,
) {
  if (id === "customTranslation" && storage === "environment") {
    throw new RuntimeControlContractError(
      path,
      "Custom Translation credentials require the system credential store",
    );
  }
}

function assertExactCredentialIds(
  credentials: readonly Readonly<{ id: CredentialId }>[],
  expectedIds: readonly CredentialId[],
  path: string,
) {
  const actualIds = new Set(credentials.map((credential) => credential.id));
  if (
    actualIds.size !== credentials.length ||
    credentials.length !== expectedIds.length ||
    expectedIds.some((id) => !actualIds.has(id))
  ) {
    throw new RuntimeControlContractError(
      path,
      "expected exactly one entry for every required credential identity",
    );
  }
}

function decodeRuntimeStatus(value: unknown, path: string): RuntimeStatusEvent {
  const input = exactRecord(value, path, ["status", "message", "timestampMs"]);
  const status = literal<RuntimeStatus>(
    input["status"],
    `${path}.status`,
    RUNTIME_STATUSES,
  );
  const timestampMs = safeInteger(
    input["timestampMs"],
    `${path}.timestampMs`,
    0,
  );

  if (input["message"] === undefined) {
    return { status, timestampMs };
  }

  return {
    status,
    message: string(input["message"], `${path}.message`),
    timestampMs,
  };
}

function decodeRuntimeGenerationCredentialSnapshot(
  value: unknown,
  path: string,
): RuntimeGenerationCredentialSnapshot {
  const input = exactRecord(value, path, [
    "id",
    "storage",
    "displaySuffix",
    "revision",
  ]);

  const id = literal<CredentialId>(input["id"], `${path}.id`, CREDENTIAL_IDS);
  const storage = literal<CredentialStorage>(
    input["storage"],
    `${path}.storage`,
    CREDENTIAL_STORAGES,
  );
  assertCredentialStorage(id, storage, `${path}.storage`);

  return {
    id,
    storage,
    displaySuffix: nullableString(
      input["displaySuffix"],
      `${path}.displaySuffix`,
    ),
    revision: safeInteger(input["revision"], `${path}.revision`, 0),
  };
}

function decodeChatboxPublication(
  value: unknown,
  path: string,
): ChatboxPublicationSnapshot {
  const tagged = record(value, path);
  const state = literal(
    tagged["state"],
    `${path}.state`,
    CHATBOX_PUBLICATION_STATES,
  );
  const fields =
    state === "unavailable"
      ? ["state", "host", "port", "reasonCode"]
      : ["state", "host", "port"];
  const input = exactRecord(value, path, fields);
  const host = string(input["host"], `${path}.host`);
  const port = safeInteger(input["port"], `${path}.port`, 0, 65_535);

  switch (state) {
    case "disabled":
    case "ready":
      return { state, host, port };
    case "unavailable":
      return {
        state,
        host,
        port,
        reasonCode: string(input["reasonCode"], `${path}.reasonCode`),
      };
  }
}

function decodeRuntimeGenerationSnapshot(
  value: unknown,
  path: string,
): RuntimeGenerationSnapshot {
  const input = exactRecord(value, path, [
    "id",
    "phase",
    "startedFromConfigRevision",
    "selection",
    "captionPipelinePlan",
    "credentials",
    "chatboxPublication",
    "uploadsMicrophoneAudio",
    "uploadsSourceText",
  ]);
  const selectionInput = exactRecord(input["selection"], `${path}.selection`, [
    "audio",
    "recognition",
    "translation",
    "osc",
    "publication",
  ]);
  const selection = {
    audio: decodeAudioConfig(
      selectionInput["audio"],
      `${path}.selection.audio`,
    ),
    recognition: decodeRecognitionConfig(
      selectionInput["recognition"],
      `${path}.selection.recognition`,
    ),
    translation:
      selectionInput["translation"] === null
        ? null
        : decodeTranslationConfig(
            selectionInput["translation"],
            `${path}.selection.translation`,
          ),
    osc: decodeOscConfig(selectionInput["osc"], `${path}.selection.osc`),
    publication: decodePublicationConfig(
      selectionInput["publication"],
      `${path}.selection.publication`,
    ),
  };
  const translationIsActive = selection.publication.content !== "sourceOnly";
  if (translationIsActive !== (selection.translation !== null)) {
    throw new RuntimeControlContractError(
      `${path}.selection.translation`,
      translationIsActive
        ? "translation content requires an effective Translation selection"
        : "Source-only content requires Translation to remain dormant",
    );
  }
  const captionPipelinePlan = decodeCaptionPipelinePlan(
    input["captionPipelinePlan"],
    `${path}.captionPipelinePlan`,
    selection.publication,
    selection.translation,
  );
  if (captionPipelinePlan.publication.state !== "compatible") {
    throw new RuntimeControlContractError(
      `${path}.captionPipelinePlan.publication.state`,
      "installed generations require a compatible publication plan",
    );
  }
  const credentials = array(input["credentials"], `${path}.credentials`).map(
    (credential, index) =>
      decodeRuntimeGenerationCredentialSnapshot(
        credential,
        `${path}.credentials[${String(index)}]`,
      ),
  );
  const expectedCredentialIds: readonly CredentialId[] =
    selection.translation?.endpoint.kind === "custom"
      ? ["openai", "customTranslation"]
      : ["openai"];
  assertExactCredentialIds(
    credentials,
    expectedCredentialIds,
    `${path}.credentials`,
  );
  const uploadsSourceText = boolean(
    input["uploadsSourceText"],
    `${path}.uploadsSourceText`,
  );
  if (uploadsSourceText !== (selection.translation !== null)) {
    throw new RuntimeControlContractError(
      `${path}.uploadsSourceText`,
      "expected disclosure to match effective Translation selection",
    );
  }

  return {
    id: safeInteger(input["id"], `${path}.id`, 1),
    phase: literal<RuntimeGenerationPhase>(
      input["phase"],
      `${path}.phase`,
      RUNTIME_GENERATION_PHASES,
    ),
    startedFromConfigRevision: safeInteger(
      input["startedFromConfigRevision"],
      `${path}.startedFromConfigRevision`,
      0,
    ),
    selection,
    captionPipelinePlan,
    credentials,
    chatboxPublication: decodeChatboxPublication(
      input["chatboxPublication"],
      `${path}.chatboxPublication`,
    ),
    uploadsMicrophoneAudio: boolean(
      input["uploadsMicrophoneAudio"],
      `${path}.uploadsMicrophoneAudio`,
    ),
    uploadsSourceText,
  };
}

export function decodeRuntimeControlSnapshot(
  value: unknown,
): RuntimeControlSnapshot {
  const input = exactRecord(value, "$", [
    "contractVersion",
    "revision",
    "runtimeStatus",
    "desired",
    "generation",
    "pendingGenerationChanges",
  ]);
  if (input["contractVersion"] !== RUNTIME_CONTROL_CONTRACT_VERSION) {
    throw new RuntimeControlContractError(
      "$.contractVersion",
      `expected ${String(RUNTIME_CONTROL_CONTRACT_VERSION)}`,
    );
  }
  const desiredInput = exactRecord(input["desired"], "$.desired", [
    "revision",
    "config",
    "captionPipelinePlan",
    "credentials",
  ]);
  const config = decodeAppConfig(desiredInput["config"], "$.desired.config");
  const credentials = array(
    desiredInput["credentials"],
    "$.desired.credentials",
  ).map((status, index) =>
    decodeCredentialStatus(status, `$.desired.credentials[${String(index)}]`),
  );
  assertExactCredentialIds(
    credentials,
    CREDENTIAL_IDS,
    "$.desired.credentials",
  );

  return {
    contractVersion: RUNTIME_CONTROL_CONTRACT_VERSION,
    revision: safeInteger(input["revision"], "$.revision", 0),
    runtimeStatus: decodeRuntimeStatus(
      input["runtimeStatus"],
      "$.runtimeStatus",
    ),
    desired: {
      revision: safeInteger(desiredInput["revision"], "$.desired.revision", 0),
      config,
      captionPipelinePlan: decodeCaptionPipelinePlan(
        desiredInput["captionPipelinePlan"],
        "$.desired.captionPipelinePlan",
        config.publication,
        config.translation,
      ),
      credentials,
    },
    generation:
      input["generation"] === null
        ? null
        : decodeRuntimeGenerationSnapshot(input["generation"], "$.generation"),
    pendingGenerationChanges: array(
      input["pendingGenerationChanges"],
      "$.pendingGenerationChanges",
    ).map((change, index) =>
      literal<RuntimePendingGenerationChange>(
        change,
        `$.pendingGenerationChanges[${String(index)}]`,
        RUNTIME_PENDING_GENERATION_CHANGES,
      ),
    ),
  };
}

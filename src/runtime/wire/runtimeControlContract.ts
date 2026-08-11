import { createDecoders } from "./contractDecoding";
import { APP_CONFIG_SCHEMA_VERSION, type AppConfig } from "../appConfig";
import { CAPTION_LANES, type CaptionLane } from "../captionAggregate";
import {
  CAPTION_BOUNDARY_OWNERS,
  CAPTION_UNIT_BEHAVIORS,
  LANE_UPDATE_BEHAVIORS,
  PUBLICATION_INCOMPATIBILITY_REASONS,
  PUBLICATION_MODES,
  PUBLICATION_PLAN_STATES,
  RECOGNITION_INPUT_SHAPES,
  RECOGNITION_PATHS,
  RESOLVED_PUBLICATION_TIMINGS,
  REVISION_BEHAVIORS,
  type CaptionBoundaryOwner,
  type CaptionPipelinePlan,
  type CaptionUnitBehavior,
  type LaneUpdateBehavior,
  type PublicationMode,
  type PublicationPlan,
  type RecognitionCapabilityProfile,
  type RecognitionInputShape,
  type RecognitionPath,
  type ResolvedPublicationTiming,
  type RevisionBehavior,
} from "../captionPipeline";
import {
  CHATBOX_PUBLICATION_STATES,
  CREDENTIAL_IDS,
  CREDENTIAL_STATUS_STATES,
  CREDENTIAL_STORAGES,
  RUNTIME_GENERATION_PHASES,
  RUNTIME_PENDING_GENERATION_CHANGES,
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
  const input = exactRecord(value, path, ["mode"]);

  return {
    mode: literal<PublicationMode>(
      input["mode"],
      `${path}.mode`,
      PUBLICATION_MODES,
    ),
  };
}

function decodeAppConfig(value: unknown, path: string): AppConfig {
  const input = exactRecord(value, path, [
    "schemaVersion",
    "audio",
    "recognition",
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

  return {
    schemaVersion: APP_CONFIG_SCHEMA_VERSION,
    audio: decodeAudioConfig(input["audio"], `${path}.audio`),
    recognition: decodeRecognitionConfig(
      input["recognition"],
      `${path}.recognition`,
    ),
    osc: decodeOscConfig(input["osc"], `${path}.osc`),
    publication: decodePublicationConfig(
      input["publication"],
      `${path}.publication`,
    ),
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
    lanes: array(input["lanes"], `${path}.lanes`).map((lane, index) => {
      const lanePath = `${path}.lanes[${String(index)}]`;
      const decoded = exactRecord(lane, lanePath, [
        "lane",
        "updates",
        "revisions",
      ]);

      return {
        lane: literal<CaptionLane>(
          decoded["lane"],
          `${lanePath}.lane`,
          CAPTION_LANES,
        ),
        updates: literal<LaneUpdateBehavior>(
          decoded["updates"],
          `${lanePath}.updates`,
          LANE_UPDATE_BEHAVIORS,
        ),
        revisions: literal<RevisionBehavior>(
          decoded["revisions"],
          `${lanePath}.revisions`,
          REVISION_BEHAVIORS,
        ),
      };
    }),
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
  expectedMode: PublicationMode,
): CaptionPipelinePlan {
  const input = exactRecord(value, path, ["recognition", "publication"]);

  return {
    recognition: decodeRecognitionProfile(
      input["recognition"],
      `${path}.recognition`,
    ),
    publication: decodePublicationPlan(
      input["publication"],
      `${path}.publication`,
      expectedMode,
    ),
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
      return {
        state,
        id: literal<CredentialId>(input["id"], `${path}.id`, CREDENTIAL_IDS),
        storage: literal<CredentialStorage>(
          input["storage"],
          `${path}.storage`,
          CREDENTIAL_STORAGES,
        ),
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

  return {
    id: literal<CredentialId>(input["id"], `${path}.id`, CREDENTIAL_IDS),
    storage: literal<CredentialStorage>(
      input["storage"],
      `${path}.storage`,
      CREDENTIAL_STORAGES,
    ),
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
    "credential",
    "chatboxPublication",
    "uploadsMicrophoneAudio",
  ]);
  const selectionInput = exactRecord(input["selection"], `${path}.selection`, [
    "audio",
    "recognition",
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
    osc: decodeOscConfig(selectionInput["osc"], `${path}.selection.osc`),
    publication: decodePublicationConfig(
      selectionInput["publication"],
      `${path}.selection.publication`,
    ),
  };
  const captionPipelinePlan = decodeCaptionPipelinePlan(
    input["captionPipelinePlan"],
    `${path}.captionPipelinePlan`,
    selection.publication.mode,
  );
  if (captionPipelinePlan.publication.state !== "compatible") {
    throw new RuntimeControlContractError(
      `${path}.captionPipelinePlan.publication.state`,
      "installed generations require a compatible publication plan",
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
    credential:
      input["credential"] === null
        ? null
        : decodeRuntimeGenerationCredentialSnapshot(
            input["credential"],
            `${path}.credential`,
          ),
    chatboxPublication: decodeChatboxPublication(
      input["chatboxPublication"],
      `${path}.chatboxPublication`,
    ),
    uploadsMicrophoneAudio: boolean(
      input["uploadsMicrophoneAudio"],
      `${path}.uploadsMicrophoneAudio`,
    ),
  };
}

export function decodeRuntimeControlSnapshotV4(
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
  if (input["contractVersion"] !== 4) {
    throw new RuntimeControlContractError("$.contractVersion", "expected 4");
  }
  const desiredInput = exactRecord(input["desired"], "$.desired", [
    "revision",
    "config",
    "captionPipelinePlan",
    "credentials",
  ]);
  const config = decodeAppConfig(desiredInput["config"], "$.desired.config");

  return {
    contractVersion: 4,
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
        config.publication.mode,
      ),
      credentials: array(
        desiredInput["credentials"],
        "$.desired.credentials",
      ).map((status, index) =>
        decodeCredentialStatus(
          status,
          `$.desired.credentials[${String(index)}]`,
        ),
      ),
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

import {
  APP_CONFIG_SCHEMA_VERSION,
  OPENAI_TRANSCRIPTION_MODELS,
  STT_PROVIDERS,
  type AppConfig,
  type BoundaryOwner,
  type CaptionLane,
  type CaptionUnitBehavior,
  type LaneUpdateBehavior,
  type OpenAiTranscriptionModel,
  type ProviderSecretStatus,
  type ProviderSecretStorage,
  type PublicationMode,
  type PublicationPlan,
  type RecognitionCapabilityProfile,
  type RecognitionInputShape,
  type RecognitionPath,
  type ResolvedPublicationPolicy,
  type RevisionBehavior,
  type RuntimeControlSnapshot,
  type RuntimePendingChange,
  type RuntimePlan,
  type RuntimeSession,
  type RuntimeSessionChatbox,
  type RuntimeSessionCredential,
  type RuntimeSessionPhase,
  type RuntimeStatus,
  type RuntimeStatusEvent,
  type SttProvider,
} from "./types";

const PUBLICATION_MODES = ["completed", "live"] as const;
const CAPTION_LANES = ["source", "translation"] as const;
const RECOGNITION_PATHS = [
  "openAiGptTranscribe",
  "openAiGptLiveTranscribe",
] as const;
const RECOGNITION_INPUT_SHAPES = ["continuousAudioFrames"] as const;
const BOUNDARY_OWNERS = ["application"] as const;
const CAPTION_UNIT_BEHAVIORS = ["unitBased"] as const;
const LANE_UPDATE_BEHAVIORS = ["completedOnly", "ongoingAndCompleted"] as const;
const REVISION_BEHAVIORS = ["appendOnly", "revisableFullSnapshot"] as const;
const RUNTIME_STATUSES = [
  "idle",
  "starting",
  "running",
  "reconnecting",
  "stopping",
  "stopped",
  "error",
] as const;
const RUNTIME_SESSION_PHASES = [
  "starting",
  "running",
  "reconnecting",
  "stopping",
  "error",
] as const;
const PROVIDER_SECRET_STORAGES = [
  "systemCredentialStore",
  "environment",
] as const;
const RUNTIME_PENDING_CHANGES = [
  "microphone",
  "recognition",
  "credential",
  "chatboxOutput",
  "publication",
] as const;

export class RuntimeControlContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`Invalid runtime control payload at ${path}: ${expectation}.`);
    this.name = "RuntimeControlContractError";
  }
}

function record(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new RuntimeControlContractError(path, "expected an object");
  }

  return value as Record<string, unknown>;
}

function exactRecord(
  value: unknown,
  path: string,
  allowedFields: readonly string[],
): Record<string, unknown> {
  const decoded = record(value, path);
  const allowed = new Set(allowedFields);
  const unknownField = Object.keys(decoded).find(
    (field) => !allowed.has(field),
  );

  if (unknownField !== undefined) {
    throw new RuntimeControlContractError(
      `${path}.${unknownField}`,
      "unknown field",
    );
  }

  return decoded;
}

function array(value: unknown, path: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new RuntimeControlContractError(path, "expected an array");
  }

  return value;
}

function string(value: unknown, path: string): string {
  if (typeof value !== "string") {
    throw new RuntimeControlContractError(path, "expected a string");
  }

  return value;
}

function boolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    throw new RuntimeControlContractError(path, "expected a boolean");
  }

  return value;
}

function safeInteger(
  value: unknown,
  path: string,
  minimum: number,
  maximum = Number.MAX_SAFE_INTEGER,
): number {
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw new RuntimeControlContractError(
      path,
      `expected a safe integer from ${String(minimum)} to ${String(maximum)}`,
    );
  }

  return value;
}

function literal<const Value extends string>(
  value: unknown,
  path: string,
  allowed: readonly Value[],
): Value {
  if (typeof value !== "string" || !allowed.includes(value as Value)) {
    throw new RuntimeControlContractError(
      path,
      `expected one of ${allowed.join(", ")}`,
    );
  }

  return value as Value;
}

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

function decodeSttConfig(value: unknown, path: string): AppConfig["stt"] {
  const input = exactRecord(value, path, ["provider", "languages", "model"]);

  return {
    provider: literal<SttProvider>(
      input["provider"],
      `${path}.provider`,
      STT_PROVIDERS,
    ),
    languages: languageHints(input["languages"], `${path}.languages`),
    model: literal<OpenAiTranscriptionModel>(
      input["model"],
      `${path}.model`,
      OPENAI_TRANSCRIPTION_MODELS,
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
    "stt",
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
  const ui = exactRecord(input["ui"], `${path}.ui`, ["showPartial"]);

  return {
    schemaVersion: APP_CONFIG_SCHEMA_VERSION,
    audio: decodeAudioConfig(input["audio"], `${path}.audio`),
    stt: decodeSttConfig(input["stt"], `${path}.stt`),
    osc: decodeOscConfig(input["osc"], `${path}.osc`),
    publication: decodePublicationConfig(
      input["publication"],
      `${path}.publication`,
    ),
    ui: {
      showPartial: boolean(ui["showPartial"], `${path}.ui.showPartial`),
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
    "boundaryOwner",
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
    boundaryOwner: literal<BoundaryOwner>(
      input["boundaryOwner"],
      `${path}.boundaryOwner`,
      BOUNDARY_OWNERS,
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

function decodeResolvedPolicy(
  value: unknown,
  path: string,
): ResolvedPublicationPolicy {
  const tagged = record(value, path);
  const policy = string(tagged["policy"], `${path}.policy`);

  switch (policy) {
    case "completed": {
      exactRecord(value, path, ["policy"]);
      return { policy };
    }
    case "liveUnit": {
      const input = exactRecord(value, path, ["policy", "observationWindowMs"]);
      return {
        policy,
        observationWindowMs: safeInteger(
          input["observationWindowMs"],
          `${path}.observationWindowMs`,
          1,
        ),
      };
    }
    default:
      throw new RuntimeControlContractError(
        `${path}.policy`,
        "expected one of completed, liveUnit",
      );
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
  const state = string(tagged["state"], `${path}.state`);

  if (state === "ready") {
    const input = exactRecord(value, path, [
      "state",
      "mode",
      "policy",
      "selectedLanes",
    ]);
    const mode = literal<PublicationMode>(
      input["mode"],
      `${path}.mode`,
      PUBLICATION_MODES,
    );
    const policy = decodeResolvedPolicy(input["policy"], `${path}.policy`);
    assertPlanMatchesMode(mode, expectedMode, `${path}.mode`);
    if (
      (mode === "completed" && policy.policy !== "completed") ||
      (mode === "live" && policy.policy === "completed")
    ) {
      throw new RuntimeControlContractError(
        `${path}.policy`,
        `policy does not implement ${mode} mode`,
      );
    }

    return {
      state,
      mode,
      policy,
      selectedLanes: decodeCaptionLanes(
        input["selectedLanes"],
        `${path}.selectedLanes`,
      ),
    };
  }

  if (state === "incompatible") {
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
    assertPlanMatchesMode(requestedMode, expectedMode, `${path}.requestedMode`);
    const reasonPath = `${path}.reason`;
    const taggedReason = record(input["reason"], reasonPath);
    const reason = string(taggedReason["reason"], `${reasonPath}.reason`);
    const decodedReason = (() => {
      if (reason === "noLanesSelected") {
        exactRecord(input["reason"], reasonPath, ["reason"]);
        return { reason } as const;
      }
      if (reason === "laneUnavailable" || reason === "modeUnsupported") {
        const decoded = exactRecord(input["reason"], reasonPath, [
          "reason",
          "lanes",
        ]);
        return {
          reason,
          lanes: decodeCaptionLanes(decoded["lanes"], `${reasonPath}.lanes`),
        } as const;
      }

      throw new RuntimeControlContractError(
        `${reasonPath}.reason`,
        "expected one of noLanesSelected, laneUnavailable, modeUnsupported",
      );
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

  throw new RuntimeControlContractError(
    `${path}.state`,
    "expected one of ready, incompatible",
  );
}

function decodeRuntimePlan(
  value: unknown,
  path: string,
  expectedMode: PublicationMode,
): RuntimePlan {
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

function decodeProviderSecretStatus(
  value: unknown,
  path: string,
): ProviderSecretStatus {
  const input = exactRecord(value, path, [
    "provider",
    "configured",
    "storage",
    "displaySuffix",
    "error",
  ]);
  const storage = input["storage"];

  return {
    provider: literal<SttProvider>(
      input["provider"],
      `${path}.provider`,
      STT_PROVIDERS,
    ),
    configured: boolean(input["configured"], `${path}.configured`),
    storage:
      storage === null
        ? null
        : literal<ProviderSecretStorage>(
            storage,
            `${path}.storage`,
            PROVIDER_SECRET_STORAGES,
          ),
    displaySuffix: nullableString(
      input["displaySuffix"],
      `${path}.displaySuffix`,
    ),
    error: nullableString(input["error"], `${path}.error`),
  };
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

function decodeRuntimeCredential(
  value: unknown,
  path: string,
): RuntimeSessionCredential {
  const input = exactRecord(value, path, [
    "provider",
    "storage",
    "displaySuffix",
    "revision",
  ]);

  return {
    provider: literal<SttProvider>(
      input["provider"],
      `${path}.provider`,
      STT_PROVIDERS,
    ),
    storage: literal<ProviderSecretStorage>(
      input["storage"],
      `${path}.storage`,
      PROVIDER_SECRET_STORAGES,
    ),
    displaySuffix: nullableString(
      input["displaySuffix"],
      `${path}.displaySuffix`,
    ),
    revision: safeInteger(input["revision"], `${path}.revision`, 0),
  };
}

function decodeRuntimeChatbox(
  value: unknown,
  path: string,
): RuntimeSessionChatbox {
  const tagged = record(value, path);
  const state = string(tagged["state"], `${path}.state`);
  const fields =
    state === "unavailable"
      ? ["state", "host", "port", "reasonCode"]
      : ["state", "host", "port"];
  const input = exactRecord(value, path, fields);
  const host = string(input["host"], `${path}.host`);
  const port = safeInteger(input["port"], `${path}.port`, 0, 65_535);

  if (state === "disabled" || state === "ready") {
    return { state, host, port };
  }
  if (state === "unavailable") {
    return {
      state,
      host,
      port,
      reasonCode: string(input["reasonCode"], `${path}.reasonCode`),
    };
  }

  throw new RuntimeControlContractError(
    `${path}.state`,
    "expected one of disabled, ready, unavailable",
  );
}

function decodeRuntimeSession(value: unknown, path: string): RuntimeSession {
  const input = exactRecord(value, path, [
    "generation",
    "phase",
    "startedFromConfigRevision",
    "selected",
    "runtimePlan",
    "credential",
    "chatbox",
    "uploadsMicrophoneAudio",
  ]);
  const selectedInput = exactRecord(input["selected"], `${path}.selected`, [
    "audio",
    "stt",
    "osc",
    "publication",
  ]);
  const selected = {
    audio: decodeAudioConfig(selectedInput["audio"], `${path}.selected.audio`),
    stt: decodeSttConfig(selectedInput["stt"], `${path}.selected.stt`),
    osc: decodeOscConfig(selectedInput["osc"], `${path}.selected.osc`),
    publication: decodePublicationConfig(
      selectedInput["publication"],
      `${path}.selected.publication`,
    ),
  };
  const runtimePlan = decodeRuntimePlan(
    input["runtimePlan"],
    `${path}.runtimePlan`,
    selected.publication.mode,
  );
  if (runtimePlan.publication.state !== "ready") {
    throw new RuntimeControlContractError(
      `${path}.runtimePlan.publication.state`,
      "installed sessions require a ready publication plan",
    );
  }

  return {
    generation: safeInteger(input["generation"], `${path}.generation`, 1),
    phase: literal<RuntimeSessionPhase>(
      input["phase"],
      `${path}.phase`,
      RUNTIME_SESSION_PHASES,
    ),
    startedFromConfigRevision: safeInteger(
      input["startedFromConfigRevision"],
      `${path}.startedFromConfigRevision`,
      0,
    ),
    selected,
    runtimePlan,
    credential:
      input["credential"] === null
        ? null
        : decodeRuntimeCredential(input["credential"], `${path}.credential`),
    chatbox: decodeRuntimeChatbox(input["chatbox"], `${path}.chatbox`),
    uploadsMicrophoneAudio: boolean(
      input["uploadsMicrophoneAudio"],
      `${path}.uploadsMicrophoneAudio`,
    ),
  };
}

export function decodeRuntimeControlSnapshotV3(
  value: unknown,
): RuntimeControlSnapshot {
  const input = exactRecord(value, "$", [
    "contractVersion",
    "revision",
    "runtime",
    "desired",
    "session",
    "pendingChanges",
  ]);
  if (input["contractVersion"] !== 3) {
    throw new RuntimeControlContractError("$.contractVersion", "expected 3");
  }
  const desiredInput = exactRecord(input["desired"], "$.desired", [
    "revision",
    "config",
    "runtimePlan",
    "providerSecrets",
  ]);
  const config = decodeAppConfig(desiredInput["config"], "$.desired.config");

  return {
    contractVersion: 3,
    revision: safeInteger(input["revision"], "$.revision", 0),
    runtime: decodeRuntimeStatus(input["runtime"], "$.runtime"),
    desired: {
      revision: safeInteger(desiredInput["revision"], "$.desired.revision", 0),
      config,
      runtimePlan: decodeRuntimePlan(
        desiredInput["runtimePlan"],
        "$.desired.runtimePlan",
        config.publication.mode,
      ),
      providerSecrets: array(
        desiredInput["providerSecrets"],
        "$.desired.providerSecrets",
      ).map((status, index) =>
        decodeProviderSecretStatus(
          status,
          `$.desired.providerSecrets[${String(index)}]`,
        ),
      ),
    },
    session:
      input["session"] === null
        ? null
        : decodeRuntimeSession(input["session"], "$.session"),
    pendingChanges: array(input["pendingChanges"], "$.pendingChanges").map(
      (change, index) =>
        literal<RuntimePendingChange>(
          change,
          `$.pendingChanges[${String(index)}]`,
          RUNTIME_PENDING_CHANGES,
        ),
    ),
  };
}

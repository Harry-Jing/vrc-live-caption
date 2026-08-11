import {
  CAPTION_LANES,
  CAPTION_STATES,
  CAPTION_AGGREGATE_CONTRACT_VERSION,
  type CaptionAggregateSnapshot,
  type CaptionLane,
  type CaptionSnapshot,
  type CaptionState,
  type SourceSnapshotRef,
} from "../captionAggregate";
import { createDecoders } from "./contractDecoding";

export class CaptionAggregateContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`Invalid caption aggregate payload at ${path}: ${expectation}.`);
    this.name = "CaptionAggregateContractError";
  }
}

const { exactRecord, array, string, safeInteger, literal } = createDecoders(
  CaptionAggregateContractError,
);

function nonEmptyString(value: unknown, path: string): string {
  const decoded = string(value, path);

  if (decoded.length === 0) {
    throw new CaptionAggregateContractError(
      path,
      "expected a non-empty string",
    );
  }

  return decoded;
}

function explicitNullableString(value: unknown, path: string) {
  return value === null ? null : nonEmptyString(value, path);
}

function explicitNullableInteger(value: unknown, path: string) {
  return value === null ? null : safeInteger(value, path, 0);
}

function decodeSourceRef(
  value: unknown,
  path: string,
): SourceSnapshotRef | null {
  if (value === null) {
    return null;
  }

  const input = exactRecord(value, path, [
    "generation",
    "streamId",
    "unitId",
    "revision",
  ]);

  return {
    generation: safeInteger(input["generation"], `${path}.generation`, 1),
    streamId: nonEmptyString(input["streamId"], `${path}.streamId`),
    unitId: nonEmptyString(input["unitId"], `${path}.unitId`),
    revision: safeInteger(input["revision"], `${path}.revision`, 1),
  };
}

function decodeCaption(value: unknown, index: number): CaptionSnapshot {
  const path = `$.captions[${String(index)}]`;
  const input = exactRecord(value, path, [
    "generation",
    "streamId",
    "unitId",
    "lane",
    "revision",
    "text",
    "state",
    "language",
    "sourceRef",
    "unitStartedAtMs",
    "timestampMs",
  ]);
  const unitId = explicitNullableString(input["unitId"], `${path}.unitId`);
  const state = literal<CaptionState>(
    input["state"],
    `${path}.state`,
    CAPTION_STATES,
  );

  if (state === "completed" && unitId === null) {
    throw new CaptionAggregateContractError(
      `${path}.unitId`,
      "completed captions require a caption unit",
    );
  }

  return {
    generation: safeInteger(input["generation"], `${path}.generation`, 1),
    streamId: nonEmptyString(input["streamId"], `${path}.streamId`),
    unitId,
    lane: literal<CaptionLane>(input["lane"], `${path}.lane`, CAPTION_LANES),
    revision: safeInteger(input["revision"], `${path}.revision`, 1),
    text: string(input["text"], `${path}.text`),
    state,
    language: explicitNullableString(input["language"], `${path}.language`),
    sourceRef: decodeSourceRef(input["sourceRef"], `${path}.sourceRef`),
    unitStartedAtMs: explicitNullableInteger(
      input["unitStartedAtMs"],
      `${path}.unitStartedAtMs`,
    ),
    timestampMs: safeInteger(input["timestampMs"], `${path}.timestampMs`, 0),
  };
}

function captionScopeKey(caption: CaptionSnapshot) {
  return JSON.stringify([
    caption.generation,
    caption.streamId,
    caption.unitId,
    caption.lane,
  ]);
}

function sourceReferenceMatches(
  caption: CaptionSnapshot,
  source: CaptionSnapshot,
) {
  const sourceRef = caption.sourceRef;

  return (
    sourceRef !== null &&
    source.lane === "source" &&
    source.state === "completed" &&
    source.generation === sourceRef.generation &&
    source.streamId === sourceRef.streamId &&
    source.unitId === sourceRef.unitId &&
    source.revision === sourceRef.revision
  );
}

export function decodeCaptionAggregateSnapshot(
  value: unknown,
): CaptionAggregateSnapshot {
  const input = exactRecord(value, "$", [
    "contractVersion",
    "snapshotRevision",
    "activeStream",
    "openSourceUnits",
    "captions",
  ]);

  if (input["contractVersion"] !== CAPTION_AGGREGATE_CONTRACT_VERSION) {
    throw new CaptionAggregateContractError(
      "$.contractVersion",
      `expected ${String(CAPTION_AGGREGATE_CONTRACT_VERSION)}`,
    );
  }

  const activeStreamInput = input["activeStream"];
  const activeStream =
    activeStreamInput === null
      ? null
      : (() => {
          const decoded = exactRecord(activeStreamInput, "$.activeStream", [
            "generation",
            "streamId",
          ]);

          return {
            generation: safeInteger(
              decoded["generation"],
              "$.activeStream.generation",
              1,
            ),
            streamId: nonEmptyString(
              decoded["streamId"],
              "$.activeStream.streamId",
            ),
          };
        })();
  const openSourceUnits = array(
    input["openSourceUnits"],
    "$.openSourceUnits",
  ).map((openSourceUnit, index) => {
    const path = `$.openSourceUnits[${String(index)}]`;
    const decoded = exactRecord(openSourceUnit, path, [
      "unitId",
      "startedAtMs",
    ]);

    return {
      unitId: nonEmptyString(decoded["unitId"], `${path}.unitId`),
      startedAtMs: safeInteger(
        decoded["startedAtMs"],
        `${path}.startedAtMs`,
        0,
      ),
    };
  });

  if (activeStream === null && openSourceUnits.length > 0) {
    throw new CaptionAggregateContractError(
      "$.openSourceUnits",
      "open source caption units require an active caption stream",
    );
  }

  if (
    new Set(openSourceUnits.map((openSourceUnit) => openSourceUnit.unitId))
      .size !== openSourceUnits.length
  ) {
    throw new CaptionAggregateContractError(
      "$.openSourceUnits",
      "caption unit identities must be unique",
    );
  }

  const captions = array(input["captions"], "$.captions").map(decodeCaption);
  const openSourceUnitIds = new Set(
    openSourceUnits.map((openSourceUnit) => openSourceUnit.unitId),
  );
  const captionKeys = new Set<string>();

  for (const caption of captions) {
    const key = captionScopeKey(caption);
    if (captionKeys.has(key)) {
      throw new CaptionAggregateContractError(
        "$.captions",
        "caption lane correlation scopes must be unique",
      );
    }
    captionKeys.add(key);

    if (
      caption.state === "ongoing" &&
      (activeStream === null ||
        caption.generation !== activeStream.generation ||
        caption.streamId !== activeStream.streamId)
    ) {
      throw new CaptionAggregateContractError(
        "$.captions",
        "ongoing captions must belong to the active caption stream",
      );
    }

    if (caption.lane === "source") {
      if (caption.sourceRef !== null) {
        throw new CaptionAggregateContractError(
          "$.captions.sourceRef",
          "source captions cannot carry a source reference",
        );
      }
      if (
        caption.state === "ongoing" &&
        caption.unitId !== null &&
        !openSourceUnitIds.has(caption.unitId)
      ) {
        throw new CaptionAggregateContractError(
          "$.openSourceUnits",
          "unitful ongoing source captions must reference an open source caption unit",
        );
      }
      if (
        caption.state === "completed" &&
        caption.unitId !== null &&
        activeStream !== null &&
        caption.generation === activeStream.generation &&
        caption.streamId === activeStream.streamId &&
        openSourceUnitIds.has(caption.unitId)
      ) {
        throw new CaptionAggregateContractError(
          "$.captions",
          "completed source caption units cannot remain open",
        );
      }
      continue;
    }

    if (
      caption.sourceRef === null ||
      caption.unitId !== caption.sourceRef.unitId ||
      caption.generation !== caption.sourceRef.generation ||
      caption.streamId !== caption.sourceRef.streamId ||
      !captions.some((source) => sourceReferenceMatches(caption, source))
    ) {
      throw new CaptionAggregateContractError(
        "$.captions.sourceRef",
        "translation captions must reference the exact retained completed source snapshot",
      );
    }
  }

  return {
    contractVersion: CAPTION_AGGREGATE_CONTRACT_VERSION,
    snapshotRevision: safeInteger(
      input["snapshotRevision"],
      "$.snapshotRevision",
      0,
    ),
    activeStream,
    openSourceUnits,
    captions,
  };
}

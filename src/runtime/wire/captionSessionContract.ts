import { createDecoders } from "./contractDecoding";
import {
  CAPTION_LANES,
  CAPTION_STATES,
  type CaptionLane,
  type CaptionSessionSnapshotV1,
  type CaptionSnapshotV1,
  type CaptionState,
} from "../types";

export class CaptionContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`Invalid caption session payload at ${path}: ${expectation}.`);
    this.name = "CaptionContractError";
  }
}

const { exactRecord, array, string, safeInteger, literal } =
  createDecoders(CaptionContractError);

function nonEmptyString(value: unknown, path: string): string {
  const decoded = string(value, path);

  if (decoded.length === 0) {
    throw new CaptionContractError(path, "expected a non-empty string");
  }

  return decoded;
}

function explicitNullableString(value: unknown, path: string) {
  return value === null ? null : nonEmptyString(value, path);
}

function explicitNullableInteger(value: unknown, path: string) {
  return value === null ? null : safeInteger(value, path, 0);
}

function decodeCaption(value: unknown, index: number): CaptionSnapshotV1 {
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
    "provider",
    "model",
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
    throw new CaptionContractError(
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
    provider: nonEmptyString(input["provider"], `${path}.provider`),
    model: nonEmptyString(input["model"], `${path}.model`),
    unitStartedAtMs: explicitNullableInteger(
      input["unitStartedAtMs"],
      `${path}.unitStartedAtMs`,
    ),
    timestampMs: safeInteger(input["timestampMs"], `${path}.timestampMs`, 0),
  };
}

export function decodeCaptionSessionSnapshotV1(
  value: unknown,
): CaptionSessionSnapshotV1 {
  const input = exactRecord(value, "$", [
    "contractVersion",
    "snapshotRevision",
    "active",
    "activeUnits",
    "captions",
  ]);

  if (input["contractVersion"] !== 1) {
    throw new CaptionContractError("$.contractVersion", "expected 1");
  }

  const activeInput = input["active"];
  const active =
    activeInput === null
      ? null
      : (() => {
          const decoded = exactRecord(activeInput, "$.active", [
            "generation",
            "streamId",
          ]);

          return {
            generation: safeInteger(
              decoded["generation"],
              "$.active.generation",
              1,
            ),
            streamId: nonEmptyString(decoded["streamId"], "$.active.streamId"),
          };
        })();
  const activeUnits = array(input["activeUnits"], "$.activeUnits").map(
    (value, index) => {
      const path = `$.activeUnits[${String(index)}]`;
      const decoded = exactRecord(value, path, ["unitId", "startedAtMs"]);

      return {
        unitId: nonEmptyString(decoded["unitId"], `${path}.unitId`),
        startedAtMs: safeInteger(
          decoded["startedAtMs"],
          `${path}.startedAtMs`,
          0,
        ),
      };
    },
  );

  if (active === null && activeUnits.length > 0) {
    throw new CaptionContractError(
      "$.activeUnits",
      "active caption units require an active recognition session",
    );
  }

  if (
    new Set(activeUnits.map((unit) => unit.unitId)).size !== activeUnits.length
  ) {
    throw new CaptionContractError(
      "$.activeUnits",
      "caption unit identities must be unique",
    );
  }

  const captions = array(input["captions"], "$.captions").map(decodeCaption);
  const activeUnitIds = new Set(activeUnits.map((unit) => unit.unitId));
  const captionKeys = new Set<string>();

  for (const caption of captions) {
    const captionKey = JSON.stringify([
      caption.generation,
      caption.streamId,
      caption.unitId,
      caption.lane,
    ]);

    if (captionKeys.has(captionKey)) {
      throw new CaptionContractError(
        "$.captions",
        "caption correlation scopes must be unique",
      );
    }
    captionKeys.add(captionKey);

    if (
      caption.state === "ongoing" &&
      (active === null ||
        caption.generation !== active.generation ||
        caption.streamId !== active.streamId)
    ) {
      throw new CaptionContractError(
        "$.captions",
        "ongoing captions must belong to the active generation and stream",
      );
    }

    if (
      caption.state === "ongoing" &&
      caption.unitId !== null &&
      !activeUnitIds.has(caption.unitId)
    ) {
      throw new CaptionContractError(
        "$.activeUnits",
        "unitful ongoing captions must reference an active caption unit",
      );
    }

    if (
      caption.state === "completed" &&
      caption.unitId !== null &&
      active !== null &&
      caption.generation === active.generation &&
      caption.streamId === active.streamId &&
      activeUnitIds.has(caption.unitId)
    ) {
      throw new CaptionContractError(
        "$.captions",
        "completed caption units cannot remain active",
      );
    }
  }

  return {
    contractVersion: 1,
    snapshotRevision: safeInteger(
      input["snapshotRevision"],
      "$.snapshotRevision",
      0,
    ),
    active,
    activeUnits,
    captions,
  };
}

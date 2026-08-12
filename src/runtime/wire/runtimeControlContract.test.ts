import { describe, expect, test } from "vitest";
import runtimeControlFixture from "../../../contracts/runtime-control-snapshot-v3.json?raw";
import {
  decodeRuntimeControlSnapshot,
  RuntimeControlContractError,
} from "./runtimeControlContract";

const fixtureJson = JSON.parse(runtimeControlFixture) as unknown;
const completePayload = decodeRuntimeControlSnapshot(fixtureJson);

function activeCustomTranslationPayload() {
  const generation = completePayload.generation;
  if (
    generation === null ||
    completePayload.desired.config.translation === null
  ) {
    throw new Error(
      "V3 fixture must contain a generation and saved Translation.",
    );
  }
  const publication = { mode: "completed", content: "bilingual" } as const;
  const translationProfile = {
    path: "openai/responses-completed-text",
    inputShape: "completedSourceSnapshots",
    lanes: [
      {
        lane: "translation",
        updates: "completedOnly",
        revisions: "appendOnly",
      },
    ],
  } as const;
  const publicationPlan = {
    state: "compatible",
    mode: "completed",
    timing: { timing: "completed" },
    selectedLanes: ["source", "translation"],
  } as const;
  const captionPipelinePlan = {
    ...completePayload.desired.captionPipelinePlan,
    translation: translationProfile,
    publication: publicationPlan,
  };

  return {
    ...completePayload,
    desired: {
      ...completePayload.desired,
      config: {
        ...completePayload.desired.config,
        publication,
      },
      captionPipelinePlan,
    },
    generation: {
      ...generation,
      selection: {
        ...generation.selection,
        translation: completePayload.desired.config.translation,
        publication,
      },
      captionPipelinePlan,
      credentials: [
        ...generation.credentials,
        {
          id: "customTranslation",
          storage: "systemCredentialStore",
          displaySuffix: "wxyz",
          revision: 1,
        },
      ],
      translationState: { state: "active" } as const,
      uploadsSourceText: true,
    },
  };
}

describe("decodeRuntimeControlSnapshot", () => {
  test("decodes the shared Rust-serialized fixture", () => {
    expect(decodeRuntimeControlSnapshot(fixtureJson)).toEqual(fixtureJson);
  });

  test("rejects an active Translation profile while content is Source-only", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        captionPipelinePlan: {
          ...completePayload.desired.captionPipelinePlan,
          translation: {
            path: "openai/responses-completed-text",
            inputShape: "completedSourceSnapshots",
            lanes: [
              {
                lane: "translation",
                updates: "completedOnly",
                revisions: "appendOnly",
              },
            ],
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.captionPipelinePlan.translation",
    );
  });

  test.each([
    "http://example.com/v1",
    "https://@example.com/v1",
    "https://example.com/v1?",
    "https://example.com/v1#",
    "https://example.com/v1/responses/",
    "https://example.com/v1/%72esponses",
    "https://example.com/v1/respon%73es",
    "https://example.com/v1/%",
    "not a URL",
  ])("rejects invalid Custom Translation API base URL %s", (apiBaseUrl) => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        config: {
          ...completePayload.desired.config,
          translation: {
            ...completePayload.desired.config.translation,
            endpoint: { kind: "custom", apiBaseUrl },
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.config.translation.endpoint.apiBaseUrl",
    );
  });

  test("rejects Translation content without an explicit selection", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        config: {
          ...completePayload.desired.config,
          translation: null,
          publication: {
            ...completePayload.desired.config.publication,
            content: "translationOnly",
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.config.translation",
    );
  });

  test("decodes active Custom Translation with both used credential identities", () => {
    const decoded = decodeRuntimeControlSnapshot(
      activeCustomTranslationPayload(),
    );

    expect(decoded.generation).toMatchObject({
      selection: { publication: { content: "bilingual" } },
      credentials: [{ id: "openai" }, { id: "customTranslation" }],
      translationState: { state: "active" },
      uploadsMicrophoneAudio: true,
      uploadsSourceText: true,
    });
  });

  test("decodes degraded Translation with its first stable failure reason", () => {
    const active = activeCustomTranslationPayload();
    const payload = {
      ...active,
      generation: {
        ...active.generation,
        translationState: {
          state: "degraded",
          reasonCode: "translation.provider_unavailable",
        },
      },
    };

    expect(decodeRuntimeControlSnapshot(payload).generation).toMatchObject({
      translationState: {
        state: "degraded",
        reasonCode: "translation.provider_unavailable",
      },
    });
  });

  test("decodes Official Translation with one deduplicated OpenAI credential", () => {
    const active = activeCustomTranslationPayload();
    const translation = {
      ...active.generation.selection.translation,
      endpoint: { kind: "official" },
    };
    const payload = {
      ...active,
      generation: {
        ...active.generation,
        selection: { ...active.generation.selection, translation },
        credentials: active.generation.credentials.filter(
          (credential) => credential.id === "openai",
        ),
      },
    };

    expect(decodeRuntimeControlSnapshot(payload).generation).toMatchObject({
      selection: { translation: { endpoint: { kind: "official" } } },
      credentials: [{ id: "openai" }],
      translationState: { state: "active" },
    });
  });

  test.each([
    ["inactive", { state: "active" }],
    ["active", { state: "inactive" }],
  ])(
    "rejects %s selection with a contradictory Translation state",
    (kind, state) => {
      const base =
        kind === "inactive"
          ? completePayload
          : activeCustomTranslationPayload();
      const payload = {
        ...base,
        generation: base.generation
          ? { ...base.generation, translationState: state }
          : null,
      };

      expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
        "$.generation.translationState",
      );
    },
  );

  test("rejects an unknown degraded Translation reason", () => {
    const active = activeCustomTranslationPayload();
    const payload = {
      ...active,
      generation: {
        ...active.generation,
        translationState: {
          state: "degraded",
          reasonCode: "translation.arbitrary",
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.generation.translationState.reasonCode",
    );
  });

  test("rejects Source-text upload disclosure that contradicts active Translation", () => {
    const payload = activeCustomTranslationPayload();
    payload.generation.uploadsSourceText = false;

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.generation.uploadsSourceText",
    );
  });

  test("rejects a non-null generation Translation selection for Source-only content", () => {
    const generation = completePayload.generation;
    const translation = completePayload.desired.config.translation;
    if (generation === null || translation === null) {
      throw new Error(
        "V3 fixture must contain a generation and saved Translation.",
      );
    }
    const payload = {
      ...completePayload,
      generation: {
        ...generation,
        selection: { ...generation.selection, translation },
        credentials: [
          ...generation.credentials,
          {
            id: "customTranslation",
            storage: "systemCredentialStore",
            displaySuffix: "wxyz",
            revision: 1,
          },
        ],
        uploadsSourceText: true,
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.generation.selection.translation",
    );
  });

  test("rejects active Custom Translation without its credential identity", () => {
    const payload = activeCustomTranslationPayload();
    payload.generation.credentials = payload.generation.credentials.filter(
      (credential) => credential.id !== "customTranslation",
    );

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.generation.credentials",
    );
  });

  test("rejects the old runtime control contract version", () => {
    expect(() =>
      decodeRuntimeControlSnapshot({
        ...(fixtureJson as object),
        contractVersion: 2,
      }),
    ).toThrow(
      "Invalid runtime control payload at $.contractVersion: expected 3.",
    );
  });

  test("rejects the old app config schema version", () => {
    expect(() =>
      decodeRuntimeControlSnapshot({
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: { ...completePayload.desired.config, schemaVersion: 4 },
        },
      }),
    ).toThrow(
      "Invalid runtime control payload at $.desired.config.schemaVersion: expected 2.",
    );
  });

  test.each([
    {
      name: "empty expected-language list",
      expectedLanguages: [] as string[],
      path: "$.desired.config.recognition.expectedLanguages",
    },
    {
      name: "blank expected-language hint",
      expectedLanguages: ["en", "   "],
      path: "$.desired.config.recognition.expectedLanguages[1]",
    },
    {
      name: "case-insensitive duplicate expected-language hint",
      expectedLanguages: ["en", " EN "],
      path: "$.desired.config.recognition.expectedLanguages",
    },
  ])("rejects $name", ({ expectedLanguages, path }) => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        config: {
          ...completePayload.desired.config,
          recognition: {
            ...completePayload.desired.config.recognition,
            expectedLanguages,
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(path);
  });

  test("decodes an active generation while its recognition service reconnects", () => {
    const payload = {
      ...completePayload,
      runtimeStatus: { status: "reconnecting", timestampMs: 901 },
      generation: completePayload.generation
        ? { ...completePayload.generation, phase: "reconnecting" }
        : null,
    };

    expect(decodeRuntimeControlSnapshot(payload)).toMatchObject({
      runtimeStatus: { status: "reconnecting" },
      generation: { phase: "reconnecting" },
    });
  });

  test("decodes an unconfigured service credential without configured-only fields", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        credentials: [
          { state: "unconfigured", id: "openai" },
          { state: "unconfigured", id: "customTranslation" },
        ],
      },
    };

    expect(decodeRuntimeControlSnapshot(payload).desired.credentials).toEqual([
      { state: "unconfigured", id: "openai" },
      { state: "unconfigured", id: "customTranslation" },
    ]);
  });

  test("decodes an unavailable service credential with a structured failure", () => {
    const failure = {
      code: "config.secret_failed",
      message: "System credential store is unavailable.",
    };
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        credentials: [
          { state: "unavailable", id: "openai", failure },
          { state: "unconfigured", id: "customTranslation" },
        ],
      },
    };

    expect(decodeRuntimeControlSnapshot(payload).desired.credentials).toEqual([
      { state: "unavailable", id: "openai", failure },
      { state: "unconfigured", id: "customTranslation" },
    ]);
  });

  test.each([
    {
      name: "a missing Custom Translation status",
      credentials: [{ state: "unconfigured", id: "openai" }],
    },
    {
      name: "a duplicate OpenAI status",
      credentials: [
        { state: "unconfigured", id: "openai" },
        { state: "unconfigured", id: "openai" },
      ],
    },
  ])("rejects desired credentials with $name", ({ credentials }) => {
    const payload = {
      ...completePayload,
      desired: { ...completePayload.desired, credentials },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.credentials",
    );
  });

  test("rejects credential fields that do not belong to the tagged state", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        credentials: [
          {
            state: "unavailable",
            id: "openai",
            failure: {
              code: "config.secret_failed",
              message: "System credential store is unavailable.",
            },
            configured: false,
          },
        ],
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.credentials[0].configured",
    );
  });

  test("rejects an environment-backed Custom Translation credential status", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        credentials: completePayload.desired.credentials.map((credential) =>
          credential.id === "customTranslation"
            ? { ...credential, storage: "environment" }
            : credential,
        ),
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.credentials[1].storage",
    );
  });

  test("preserves an incompatible desired publication request", () => {
    const incompatiblePublication = {
      state: "incompatible",
      requestedMode: "live",
      selectedLanes: ["source"],
      reason: { reason: "modeUnsupported", lanes: ["source"] },
      supportedModes: ["completed"],
    };
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        captionPipelinePlan: {
          ...completePayload.desired.captionPipelinePlan,
          publication: incompatiblePublication,
        },
      },
      generation: null,
    };

    expect(decodeRuntimeControlSnapshot(payload)).toMatchObject({
      desired: {
        config: { publication: { mode: "live" } },
        captionPipelinePlan: { publication: incompatiblePublication },
      },
      generation: null,
    });
  });

  test("decodes completed and unit-based Live timings", () => {
    const cases = [
      { mode: "completed", timing: { timing: "completed" } },
      {
        mode: "live",
        timing: { timing: "liveUnit", observationWindowMs: 1000 },
      },
    ] as const;

    for (const { mode, timing } of cases) {
      const payload = {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            publication: {
              ...completePayload.desired.config.publication,
              mode,
            },
          },
          captionPipelinePlan: {
            ...completePayload.desired.captionPipelinePlan,
            publication: {
              state: "compatible",
              mode,
              timing,
              selectedLanes: ["source"],
            },
          },
        },
        generation: null,
      };

      expect(
        decodeRuntimeControlSnapshot(payload).desired.captionPipelinePlan
          .publication,
      ).toEqual(payload.desired.captionPipelinePlan.publication);
    }
  });

  test.each([
    ["inputShape", "completedAudioUnits"],
    ["captionBoundaryOwner", "provider"],
    ["captionBoundaryOwner", "none"],
    ["unitBehavior", "unitless"],
  ] as const)("rejects removed recognition %s value %s", (field, value) => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        captionPipelinePlan: {
          ...completePayload.desired.captionPipelinePlan,
          recognition: {
            ...completePayload.desired.captionPipelinePlan.recognition,
            [field]: value,
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      `$.desired.captionPipelinePlan.recognition.${field}`,
    );
  });

  test("rejects removed unitless Live timing", () => {
    const payload = {
      ...completePayload,
      desired: {
        ...completePayload.desired,
        captionPipelinePlan: {
          ...completePayload.desired.captionPipelinePlan,
          publication: {
            ...completePayload.desired.captionPipelinePlan.publication,
            timing: { timing: "liveUnitless", firstNonEmptyDelayMs: 1000 },
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "$.desired.captionPipelinePlan.publication.timing.timing",
    );
  });

  test("rejects an incompatible plan on an installed generation", () => {
    const generation = completePayload.generation;
    if (generation === null) {
      throw new Error("Fixture must contain a runtime generation.");
    }
    const payload = {
      ...completePayload,
      generation: {
        ...generation,
        captionPipelinePlan: {
          ...generation.captionPipelinePlan,
          publication: {
            state: "incompatible",
            requestedMode: "live",
            selectedLanes: ["source"],
            reason: { reason: "modeUnsupported", lanes: ["source"] },
            supportedModes: ["completed"],
          },
        },
      },
    };

    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(
      "Invalid runtime control payload at $.generation.captionPipelinePlan.publication.state: installed generations require a compatible publication plan.",
    );
  });

  test.each([
    [
      "a pre-baseline contract version",
      { ...completePayload, contractVersion: 1 },
      "$.contractVersion",
    ],
    [
      "the removed stt config shape",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            recognition: undefined,
            stt: {
              provider: "openai",
              languages: ["en"],
              model: "gpt-transcribe",
            },
          },
        },
      },
      "$.desired.config.stt",
    ],
    [
      "the removed partial-preview field",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            ui: { showPartial: true },
          },
        },
      },
      "$.desired.config.ui.showPartial",
    ],
    [
      "an arbitrary recognition path",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            recognition: {
              ...completePayload.desired.config.recognition,
              path: "openai/removed-model",
            },
          },
        },
      },
      "$.desired.config.recognition.path",
    ],
    [
      "unknown nested fields",
      {
        ...completePayload,
        desired: { ...completePayload.desired, unexpected: true },
      },
      "$.desired.unexpected",
    ],
    [
      "a non-positive Live delay",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          captionPipelinePlan: {
            ...completePayload.desired.captionPipelinePlan,
            publication: {
              ...completePayload.desired.captionPipelinePlan.publication,
              timing: { timing: "liveUnit", observationWindowMs: 0 },
            },
          },
        },
      },
      "$.desired.captionPipelinePlan.publication.timing.observationWindowMs",
    ],
    [
      "a plan that disagrees with the configured mode",
      {
        ...completePayload,
        desired: {
          ...completePayload.desired,
          config: {
            ...completePayload.desired.config,
            publication: {
              ...completePayload.desired.config.publication,
              mode: "completed",
            },
          },
        },
      },
      "$.desired.captionPipelinePlan.publication.mode",
    ],
  ])("rejects %s", (_name, payload, path) => {
    expect(() => decodeRuntimeControlSnapshot(payload)).toThrow(path);
  });

  test("preserves the runtime control contract error type", () => {
    expect(() => decodeRuntimeControlSnapshot(null)).toThrow(
      RuntimeControlContractError,
    );
  });
});

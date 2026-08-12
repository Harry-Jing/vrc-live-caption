// Browser preview AppGateway adapter. It exists so the UI
// can be developed in a plain browser (`pnpm dev`) without the Tauri shell.
// Simulated activity is delivered through the same event handlers as the real
// Tauri adapter, so preview mode exercises the actual caption state machine.

import type { AppGateway, RuntimeEventListener } from "../../runtime/gateway";
import { isActiveRuntimeStatus } from "../../runtime/lifecycle";
import {
  appConfigValidationError,
  type AppConfig,
  APP_CONFIG_SCHEMA_VERSION,
} from "../../runtime/appConfig";
import {
  CAPTION_AGGREGATE_CONTRACT_VERSION,
  type CaptionAggregateSnapshot,
} from "../../runtime/captionAggregate";
import type { CaptionPipelinePlan } from "../../runtime/captionPipeline";
import {
  RUNTIME_CONTROL_CONTRACT_VERSION,
  type CredentialId,
  type CredentialStatus,
  type RuntimeControlSnapshot,
  type RuntimeGenerationSnapshot,
  type RuntimePendingGenerationChange,
} from "../../runtime/runtimeControl";
import type {
  DiagnosticCategory,
  RuntimeStatus,
  RuntimeStatusEvent,
} from "../../runtime/runtimeEvents";
import {
  createPreviewTranslationScenarioSeed,
  previewTranslationScenarioFromSearch,
} from "./translationScenarios";

const PREVIEW_DEFAULT_CONFIG: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: {
    inputDeviceId: null,
  },
  recognition: {
    path: "openai/gpt-transcribe",
    expectedLanguages: ["en"],
  },
  translation: null,
  osc: {
    host: "127.0.0.1",
    port: 9000,
    enabled: true,
  },
  publication: {
    mode: "completed",
    content: "sourceOnly",
  },
  ui: {
    showOngoingPreview: true,
  },
};

export function previewCaptionPipelinePlan(
  config: AppConfig,
): CaptionPipelinePlan {
  const recognition =
    config.recognition.path === "openai/gpt-transcribe"
      ? {
          path: "openai/gpt-transcribe" as const,
          inputShape: "continuousAudioFrames" as const,
          captionBoundaryOwner: "application" as const,
          unitBehavior: "unitBased" as const,
          lanes: [
            {
              lane: "source" as const,
              updates: "completedOnly" as const,
              revisions: "appendOnly" as const,
            },
          ],
        }
      : {
          path: "openai/gpt-live-transcribe" as const,
          inputShape: "continuousAudioFrames" as const,
          captionBoundaryOwner: "application" as const,
          unitBehavior: "unitBased" as const,
          lanes: [
            {
              lane: "source" as const,
              updates: "ongoingAndCompleted" as const,
              revisions: "revisableFullSnapshot" as const,
            },
          ],
        };
  const sourceUpdates = recognition.lanes[0]?.updates;
  if (!sourceUpdates) {
    throw new Error("Preview recognition profile must produce a source lane.");
  }
  const translation =
    config.publication.content === "sourceOnly" || config.translation === null
      ? null
      : {
          path: config.translation.path,
          inputShape: "completedSourceSnapshots" as const,
          lanes: [
            {
              lane: "translation" as const,
              updates: "completedOnly" as const,
              revisions: "appendOnly" as const,
            },
          ],
        };
  const selectedLanes =
    config.publication.content === "sourceOnly"
      ? (["source"] as const)
      : config.publication.content === "translationOnly"
        ? (["translation"] as const)
        : (["source", "translation"] as const);
  const availableUpdates = new Map([
    ["source", sourceUpdates],
    ...(translation === null
      ? []
      : ([["translation", translation.lanes[0]?.updates]] as const)),
  ]);
  const unavailableLanes = selectedLanes.filter(
    (lane) => !availableUpdates.has(lane),
  );
  const unsupportedLanes = selectedLanes.filter(
    (lane) =>
      config.publication.mode === "live" &&
      availableUpdates.get(lane) !== "ongoingAndCompleted",
  );
  const supportedModes = (["completed", "live"] as const).filter((mode) =>
    selectedLanes.every(
      (lane) =>
        availableUpdates.has(lane) &&
        (mode === "completed" ||
          availableUpdates.get(lane) === "ongoingAndCompleted"),
    ),
  );

  return {
    recognition,
    translation,
    publication:
      unavailableLanes.length > 0
        ? {
            state: "incompatible",
            requestedMode: config.publication.mode,
            selectedLanes,
            reason: { reason: "laneUnavailable", lanes: unavailableLanes },
            supportedModes: [],
          }
        : unsupportedLanes.length > 0
          ? {
              state: "incompatible",
              requestedMode: config.publication.mode,
              selectedLanes,
              reason: { reason: "modeUnsupported", lanes: unsupportedLanes },
              supportedModes,
            }
          : {
              state: "compatible",
              mode: config.publication.mode,
              timing:
                config.publication.mode === "completed"
                  ? { timing: "completed" }
                  : { timing: "liveUnit", observationWindowMs: 1000 },
              selectedLanes,
            },
  };
}

export function createPreviewAppGateway(search = ""): AppGateway {
  const requestedTranslationScenario =
    previewTranslationScenarioFromSearch(search);
  const translationScenario = requestedTranslationScenario
    ? createPreviewTranslationScenarioSeed(requestedTranslationScenario)
    : null;
  const subscriptions = new Set<Readonly<{ listener: RuntimeEventListener }>>();
  const controlSubscriptions = new Set<
    Readonly<{ listener: (snapshot: RuntimeControlSnapshot) => void }>
  >();
  let config = structuredClone(
    translationScenario?.config ?? PREVIEW_DEFAULT_CONFIG,
  );
  const credentialSuffixes: Record<CredentialId, string | null> = {
    openai: translationScenario?.credentialSuffixes.openai ?? null,
    customTranslation:
      translationScenario?.credentialSuffixes.customTranslation ?? null,
  };
  const credentialRevisions: Record<CredentialId, number> = {
    openai:
      translationScenario?.generation?.credentials.find(
        ({ id }) => id === "openai",
      )?.revision ?? 0,
    customTranslation:
      translationScenario?.generation?.credentials.find(
        ({ id }) => id === "customTranslation",
      )?.revision ?? 0,
  };
  let configRevision = 1;
  let controlRevision = 1;
  let nextGeneration = translationScenario?.generation?.id ?? 0;
  let generation: RuntimeGenerationSnapshot | null =
    translationScenario?.generation == null
      ? null
      : {
          ...structuredClone(translationScenario.generation),
          captionPipelinePlan: previewCaptionPipelinePlan(config),
        };
  let nextEventNumber = 1;
  let captionAggregate: CaptionAggregateSnapshot = structuredClone(
    translationScenario?.captionAggregate ?? {
      contractVersion: CAPTION_AGGREGATE_CONTRACT_VERSION,
      snapshotRevision: 0,
      activeStream: null,
      openSourceUnits: [],
      captions: [],
      translationUnits: [],
    },
  );
  let latestStatus: RuntimeStatusEvent = structuredClone(
    translationScenario?.runtimeStatus ?? {
      status: "idle",
      message: "Runtime is idle",
      timestampMs: Date.now(),
    },
  );

  function nextPreviewEventId(prefix: string) {
    nextEventNumber += 1;
    return `${prefix}-preview-${String(nextEventNumber)}`;
  }

  function emit(event: Parameters<RuntimeEventListener>[0]) {
    for (const subscription of subscriptions) {
      subscription.listener(event);
    }
  }

  function emitStatus(status: RuntimeStatus, message: string) {
    latestStatus = { status, message, timestampMs: Date.now() };
    emitControlSnapshot();
    emit({ type: "status", payload: latestStatus });
  }

  function emitDiagnostic(
    category: DiagnosticCategory,
    code: string,
    message: string,
    detail: string,
  ) {
    emit({
      type: "diagnostic",
      payload: {
        id: nextPreviewEventId("diagnostic"),
        category,
        severity: "info",
        code,
        message,
        detail,
        timestampMs: Date.now(),
      },
    });
  }

  function emitCaptionAggregateUpdate(
    next: Omit<
      CaptionAggregateSnapshot,
      "contractVersion" | "snapshotRevision"
    >,
  ) {
    captionAggregate = {
      contractVersion: CAPTION_AGGREGATE_CONTRACT_VERSION,
      snapshotRevision: captionAggregate.snapshotRevision + 1,
      ...next,
    };
    emit({
      type: "captionAggregateChanged",
      payload: structuredClone(captionAggregate),
    });
  }

  function credentialStatus(id: CredentialId): CredentialStatus {
    const suffix = credentialSuffixes[id];

    return suffix === null
      ? { state: "unconfigured", id }
      : {
          state: "configured",
          id,
          storage: "systemCredentialStore",
          displaySuffix: suffix,
        };
  }

  function controlSnapshot(): RuntimeControlSnapshot {
    return {
      contractVersion: RUNTIME_CONTROL_CONTRACT_VERSION,
      revision: controlRevision,
      runtimeStatus: { ...latestStatus },
      desired: {
        revision: configRevision,
        config: structuredClone(config),
        captionPipelinePlan: previewCaptionPipelinePlan(config),
        credentials: [
          credentialStatus("openai"),
          credentialStatus("customTranslation"),
        ],
      },
      generation: generation ? structuredClone(generation) : null,
      pendingGenerationChanges: pendingGenerationChanges(),
    };
  }

  function pendingGenerationChanges(): RuntimeControlSnapshot["pendingGenerationChanges"] {
    if (generation === null) {
      return [];
    }

    const pending: RuntimePendingGenerationChange[] = [];

    if (
      generation.selection.audio.inputDeviceId !== config.audio.inputDeviceId
    ) {
      pending.push("microphone");
    }
    if (
      generation.selection.recognition.expectedLanguages.length !==
        config.recognition.expectedLanguages.length ||
      generation.selection.recognition.expectedLanguages.some(
        (language, index) =>
          language !== config.recognition.expectedLanguages[index],
      ) ||
      generation.selection.recognition.path !== config.recognition.path
    ) {
      pending.push("recognition");
    }
    const desiredTranslation =
      config.publication.content === "sourceOnly" ? null : config.translation;
    if (
      generation.selection.publication.content === config.publication.content &&
      JSON.stringify(generation.selection.translation) !==
        JSON.stringify(desiredTranslation)
    ) {
      pending.push("translation");
    }
    if (
      generation.credentials.some(
        (credential) =>
          credential.revision !== credentialRevisions[credential.id],
      )
    ) {
      pending.push("credential");
    }
    if (
      generation.selection.osc.enabled !== config.osc.enabled ||
      generation.selection.osc.host !== config.osc.host ||
      generation.selection.osc.port !== config.osc.port
    ) {
      pending.push("chatboxOutput");
    }
    if (
      generation.selection.publication.mode !== config.publication.mode ||
      generation.selection.publication.content !== config.publication.content
    ) {
      pending.push("publication");
    }

    return pending;
  }

  function selectOscTestConfig() {
    return generation?.selection.osc ?? config.osc;
  }

  function emitControlSnapshot() {
    controlRevision += 1;
    const snapshot = controlSnapshot();

    for (const subscription of controlSubscriptions) {
      subscription.listener(structuredClone(snapshot));
    }
  }

  function createGeneration(
    phase: RuntimeGenerationSnapshot["phase"],
  ): RuntimeGenerationSnapshot {
    const selection = {
      audio: structuredClone(config.audio),
      recognition: structuredClone(config.recognition),
      translation:
        config.publication.content === "sourceOnly"
          ? null
          : structuredClone(config.translation),
      osc: structuredClone(config.osc),
      publication: structuredClone(config.publication),
    };

    return {
      id: nextGeneration,
      phase,
      startedFromConfigRevision: configRevision,
      selection,
      captionPipelinePlan: previewCaptionPipelinePlan(config),
      credentials: [
        {
          id: "openai",
          storage: "systemCredentialStore",
          displaySuffix: credentialSuffixes.openai,
          revision: credentialRevisions.openai,
        },
      ],
      chatboxPublication: {
        state: selection.osc.enabled ? "ready" : "disabled",
        host: selection.osc.host,
        port: selection.osc.port,
      },
      translationState: { state: "inactive" },
      uploadsMicrophoneAudio: true,
      uploadsSourceText: false,
    };
  }

  function startPreviewRuntime(): Promise<RuntimeControlSnapshot> {
    if (isActiveRuntimeStatus(latestStatus.status)) {
      return Promise.reject(
        new Error("The browser preview runtime is already active."),
      );
    }

    const captionPipelinePlan = previewCaptionPipelinePlan(config);
    if (captionPipelinePlan.publication.state === "incompatible") {
      return Promise.reject(
        new Error(
          "The selected recognition path and publication mode are incompatible.",
        ),
      );
    }
    if (captionPipelinePlan.translation !== null) {
      return Promise.reject(
        new Error(
          "The selected Translation path is not implemented yet (translation.module_unavailable).",
        ),
      );
    }
    nextGeneration += 1;
    generation = createGeneration("starting");
    emitCaptionAggregateUpdate({
      activeStream: {
        generation: nextGeneration,
        streamId: `recognition-${String(nextGeneration)}-1`,
      },
      openSourceUnits: [],
      captions: captionAggregate.captions.filter(
        (caption) => caption.state === "completed",
      ),
      translationUnits: [],
    });
    emitStatus("starting", "Starting browser preview runtime");
    generation = { ...generation, phase: "running" };
    emitStatus("running", "Browser preview runtime is running");
    emit({
      type: "audioLevel",
      payload: {
        generation: nextGeneration,
        revision: 1,
        rmsDbfs: -24,
        peakDbfs: -6,
        clipping: false,
        gateOpen: true,
        timestampMs: Date.now(),
      },
    });

    return Promise.resolve(controlSnapshot());
  }

  function stopPreviewRuntime(): Promise<RuntimeControlSnapshot> {
    if (latestStatus.status === "idle" || latestStatus.status === "stopped") {
      generation = null;
      emitStatus("stopped", "Browser preview runtime is already stopped");
      return Promise.resolve(controlSnapshot());
    }

    if (generation) {
      generation = { ...generation, phase: "stopping" };
    }
    emitStatus("stopping", "Stopping browser preview runtime");
    emitCaptionAggregateUpdate({
      activeStream: null,
      openSourceUnits: [],
      captions: captionAggregate.captions.filter(
        (caption) => caption.state === "completed",
      ),
      translationUnits: [],
    });
    generation = null;
    emitStatus("stopped", "Browser preview runtime stopped");
    emitDiagnostic(
      "runtime",
      "runtime.stopped",
      "Runtime stopped",
      "Browser preview capture has been released.",
    );

    return Promise.resolve(controlSnapshot());
  }

  return {
    subscribeRuntimeEvents(eventListener: RuntimeEventListener) {
      const subscription = { listener: eventListener };
      subscriptions.add(subscription);

      return Promise.resolve(() => {
        subscriptions.delete(subscription);
      });
    },

    subscribeRuntimeControlSnapshots(listener) {
      const subscription = { listener };
      controlSubscriptions.add(subscription);

      return Promise.resolve(() => {
        controlSubscriptions.delete(subscription);
      });
    },

    sendOscTestMessage() {
      const oscConfig = selectOscTestConfig();

      emitDiagnostic(
        "osc",
        "osc.test_simulated",
        "OSC test simulated",
        `Desktop-only OSC test to ${oscConfig.host}:${String(oscConfig.port)} was simulated for UI preview.`,
      );

      return Promise.resolve();
    },

    startRuntime: startPreviewRuntime,

    stopRuntime: stopPreviewRuntime,

    getRuntimeControlSnapshot() {
      return Promise.resolve(controlSnapshot());
    },

    getCaptionAggregateSnapshot() {
      return Promise.resolve(structuredClone(captionAggregate));
    },

    saveAppConfig(nextConfig: AppConfig) {
      const validationError = appConfigValidationError(nextConfig);
      if (validationError !== null) {
        return Promise.reject(new Error(validationError));
      }
      config = structuredClone(nextConfig);
      configRevision += 1;
      emitControlSnapshot();
      return Promise.resolve(controlSnapshot());
    },

    listAudioInputDevices() {
      return Promise.resolve([
        {
          id: "browser-preview-default",
          name: "Browser preview device",
          isDefault: true,
        },
      ]);
    },

    probeAudioInput(request) {
      return Promise.resolve({
        sampleRate: 48_000,
        durationMs: request.durationMs,
        rmsDbfs: -24,
        peakDbfs: -6,
        clipping: false,
        gateOpen: true,
      });
    },

    saveCredential(id: CredentialId, secret: string) {
      const trimmed = secret.trim();

      if (!trimmed) {
        return Promise.reject(new Error("API key cannot be empty."));
      }

      // Mirrors the desktop app's normalize_secret control-character rule.
      if (/\p{Cc}/u.test(trimmed)) {
        return Promise.reject(
          new Error("API key cannot contain control characters."),
        );
      }

      credentialSuffixes[id] = trimmed.slice(-4);
      credentialRevisions[id] += 1;
      emitControlSnapshot();
      return Promise.resolve(controlSnapshot());
    },

    deleteCredential(id: CredentialId) {
      credentialSuffixes[id] = null;
      credentialRevisions[id] += 1;
      emitControlSnapshot();

      return Promise.resolve(controlSnapshot());
    },
  };
}

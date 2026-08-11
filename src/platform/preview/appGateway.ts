// Browser preview AppGateway adapter. It exists so the UI
// can be developed in a plain browser (`pnpm dev`) without the Tauri shell.
// Simulated activity is delivered through the same event handlers as the real
// Tauri adapter, so preview mode exercises the actual caption state machine.

import type { AppGateway, RuntimeEventListener } from "../../runtime/gateway";
import { isActiveRuntimeStatus } from "../../runtime/lifecycle";
import {
  type AppConfig,
  APP_CONFIG_SCHEMA_VERSION,
} from "../../runtime/appConfig";
import type { CaptionAggregateSnapshotV2 } from "../../runtime/captionAggregate";
import type { CaptionPipelinePlan } from "../../runtime/captionPipeline";
import type {
  CredentialId,
  CredentialStatus,
  RuntimeControlSnapshot,
  RuntimeGenerationSnapshot,
  RuntimePendingGenerationChange,
} from "../../runtime/runtimeControl";
import type {
  DiagnosticCategory,
  RuntimeStatus,
  RuntimeStatusEvent,
} from "../../runtime/runtimeEvents";

const PREVIEW_DEFAULT_CONFIG: AppConfig = {
  schemaVersion: APP_CONFIG_SCHEMA_VERSION,
  audio: {
    inputDeviceId: null,
  },
  recognition: {
    path: "openai/gpt-transcribe",
    expectedLanguages: ["en"],
  },
  osc: {
    host: "127.0.0.1",
    port: 9000,
    enabled: true,
  },
  publication: {
    mode: "completed",
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
  const compatible =
    config.publication.mode === "completed" ||
    sourceUpdates === "ongoingAndCompleted";

  return {
    recognition,
    publication: compatible
      ? {
          state: "compatible",
          mode: config.publication.mode,
          timing:
            config.publication.mode === "completed"
              ? { timing: "completed" }
              : { timing: "liveUnit", observationWindowMs: 1000 },
          selectedLanes: ["source"],
        }
      : {
          state: "incompatible",
          requestedMode: config.publication.mode,
          selectedLanes: ["source"],
          reason: { reason: "modeUnsupported", lanes: ["source"] },
          supportedModes: ["completed"],
        },
  };
}

export function createPreviewAppGateway(): AppGateway {
  const subscriptions = new Set<Readonly<{ listener: RuntimeEventListener }>>();
  const controlSubscriptions = new Set<
    Readonly<{ listener: (snapshot: RuntimeControlSnapshot) => void }>
  >();
  let config = structuredClone(PREVIEW_DEFAULT_CONFIG);
  let openAiCredentialSuffix: string | null = null;
  let credentialRevision = 0;
  let configRevision = 1;
  let controlRevision = 1;
  let nextGeneration = 0;
  let generation: RuntimeGenerationSnapshot | null = null;
  let generationCredentialRevision: number | null = null;
  let nextEventNumber = 1;
  let captionAggregate: CaptionAggregateSnapshotV2 = {
    contractVersion: 2,
    snapshotRevision: 0,
    activeStream: null,
    openSourceUnits: [],
    captions: [],
  };
  let latestStatus: RuntimeStatusEvent = {
    status: "idle",
    message: "Runtime is idle",
    timestampMs: Date.now(),
  };

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
      CaptionAggregateSnapshotV2,
      "contractVersion" | "snapshotRevision"
    >,
  ) {
    captionAggregate = {
      contractVersion: 2,
      snapshotRevision: captionAggregate.snapshotRevision + 1,
      ...next,
    };
    emit({
      type: "captionAggregateChanged",
      payload: structuredClone(captionAggregate),
    });
  }

  function openAiCredentialStatus(): CredentialStatus {
    return openAiCredentialSuffix === null
      ? { state: "unconfigured", id: "openai" }
      : {
          state: "configured",
          id: "openai",
          storage: "systemCredentialStore",
          displaySuffix: openAiCredentialSuffix,
        };
  }

  function controlSnapshot(): RuntimeControlSnapshot {
    return {
      contractVersion: 4,
      revision: controlRevision,
      runtimeStatus: { ...latestStatus },
      desired: {
        revision: configRevision,
        config: structuredClone(config),
        captionPipelinePlan: previewCaptionPipelinePlan(config),
        credentials: [openAiCredentialStatus()],
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
    if (generationCredentialRevision !== credentialRevision) {
      pending.push("credential");
    }
    if (
      generation.selection.osc.enabled !== config.osc.enabled ||
      generation.selection.osc.host !== config.osc.host ||
      generation.selection.osc.port !== config.osc.port
    ) {
      pending.push("chatboxOutput");
    }
    if (generation.selection.publication.mode !== config.publication.mode) {
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
      osc: structuredClone(config.osc),
      publication: structuredClone(config.publication),
    };

    return {
      id: nextGeneration,
      phase,
      startedFromConfigRevision: configRevision,
      selection,
      captionPipelinePlan: previewCaptionPipelinePlan(config),
      credential:
        openAiCredentialSuffix !== null
          ? {
              id: "openai",
              storage: "systemCredentialStore",
              displaySuffix: openAiCredentialSuffix,
              revision: credentialRevision,
            }
          : null,
      chatboxPublication: {
        state: selection.osc.enabled ? "ready" : "disabled",
        host: selection.osc.host,
        port: selection.osc.port,
      },
      uploadsMicrophoneAudio: true,
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
    nextGeneration += 1;
    generationCredentialRevision = credentialRevision;
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
      generationCredentialRevision = null;
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
    });
    generation = null;
    generationCredentialRevision = null;
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
      void id;
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

      openAiCredentialSuffix = trimmed.slice(-4);
      credentialRevision += 1;
      emitControlSnapshot();
      return Promise.resolve(controlSnapshot());
    },

    deleteCredential(id: CredentialId) {
      void id;
      openAiCredentialSuffix = null;
      credentialRevision += 1;
      emitControlSnapshot();

      return Promise.resolve(controlSnapshot());
    },
  };
}

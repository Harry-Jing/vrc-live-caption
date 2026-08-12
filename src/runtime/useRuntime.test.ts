import { createRenderer, defineComponent } from "vue";
import { expect, test, vi } from "vitest";
import captionAggregateFixture from "../../contracts/caption-aggregate-snapshot-v2.json?raw";
import runtimeControlFixture from "../../contracts/runtime-control-snapshot-v3.json?raw";
import type {
  AppGateway,
  RuntimeControlSnapshotListener,
  RuntimeEventListener,
} from "./gateway";
import { decodeCaptionAggregateSnapshot } from "./wire/captionAggregateContract";
import { decodeRuntimeControlSnapshot } from "./wire/runtimeControlContract";
import type { CaptionAggregateSnapshot } from "./captionAggregate";
import type { RuntimeControlSnapshot } from "./runtimeControl";
import { useRuntime } from "./useRuntime";

const gatewayHarness: { current: AppGateway | undefined } = vi.hoisted(() => ({
  current: undefined,
}));

vi.mock("../platform/appGateway", () => ({
  createAppGateway: () => gatewayHarness.current,
}));

type HostNode = {
  parent: HostElement | null;
  text: string;
};

type HostElement = HostNode & {
  children: HostNode[];
};

const renderer = createRenderer<HostNode, HostElement>({
  patchProp() {},
  insert(node, parent, anchor) {
    node.parent = parent;
    const anchorIndex = anchor == null ? -1 : parent.children.indexOf(anchor);
    if (anchorIndex === -1) {
      parent.children.push(node);
    } else {
      parent.children.splice(anchorIndex, 0, node);
    }
  },
  remove(node) {
    const parent = node.parent;
    if (parent !== null) {
      const index = parent.children.indexOf(node);
      if (index !== -1) {
        parent.children.splice(index, 1);
      }
    }
    node.parent = null;
  },
  createElement() {
    return { parent: null, text: "", children: [] };
  },
  createText(text) {
    return { parent: null, text };
  },
  createComment(text) {
    return { parent: null, text };
  },
  setText(node, text) {
    node.text = text;
  },
  setElementText(node, text) {
    node.text = text;
    node.children = [];
  },
  parentNode(node) {
    return node.parent;
  },
  nextSibling(node) {
    const parent = node.parent;
    if (parent === null) {
      return null;
    }
    const index = parent.children.indexOf(node);
    return parent.children[index + 1] ?? null;
  },
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((complete, fail) => {
    resolve = complete;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function mountRuntimeHarness() {
  const fixtureControl = decodeRuntimeControlSnapshot(
    JSON.parse(runtimeControlFixture) as unknown,
  );
  const fixtureCaption = decodeCaptionAggregateSnapshot(
    JSON.parse(captionAggregateFixture) as unknown,
  );
  if (fixtureControl.generation === null) {
    throw new Error("The runtime control fixture must contain a generation.");
  }
  const fixtureGeneration = fixtureControl.generation;
  const initialControl: RuntimeControlSnapshot = {
    ...fixtureControl,
    revision: 1,
    runtimeStatus: { status: "running", timestampMs: 1_000 },
    generation: {
      ...fixtureGeneration,
      id: 7,
    },
  };
  const stoppedControl: RuntimeControlSnapshot = {
    ...initialControl,
    revision: 2,
    runtimeStatus: { status: "stopped", timestampMs: 1_100 },
    generation: null,
  };
  const restartedControl: RuntimeControlSnapshot = {
    ...initialControl,
    revision: 3,
    runtimeStatus: { status: "running", timestampMs: 900 },
    generation: {
      ...fixtureGeneration,
      id: 8,
    },
  };
  const initialCaption = {
    ...fixtureCaption,
    snapshotRevision: 1,
    activeStream: { generation: 7, streamId: "recognition-7-1" },
    openSourceUnits: [{ unitId: "speech-7-1", startedAtMs: 1_000 }],
    captions: [],
    translationUnits: [],
  };
  const stoppedCaption = {
    ...initialCaption,
    snapshotRevision: 2,
    activeStream: null,
    openSourceUnits: [],
  };
  const restartedCaption = {
    ...initialCaption,
    snapshotRevision: 3,
    activeStream: { generation: 8, streamId: "recognition-8-1" },
    openSourceUnits: [{ unitId: "speech-8-1", startedAtMs: 1_200 }],
  };
  const pendingStart = deferred<RuntimeControlSnapshot>();
  let controlListener: RuntimeControlSnapshotListener = () => {
    throw new Error("The runtime control listener is not registered.");
  };
  let eventListener: RuntimeEventListener = () => {
    throw new Error("The runtime event listener is not registered.");
  };
  let currentControl = initialControl;
  let currentCaption: CaptionAggregateSnapshot = initialCaption;
  const unsubscribeEvents = vi.fn();
  const unsubscribeControl = vi.fn();
  const startRuntime = vi.fn(() => pendingStart.promise);
  const gateway: AppGateway = {
    subscribeRuntimeEvents(listener) {
      eventListener = listener;
      return Promise.resolve(unsubscribeEvents);
    },
    subscribeRuntimeControlSnapshots(listener) {
      controlListener = listener;
      return Promise.resolve(unsubscribeControl);
    },
    sendOscTestMessage: () => Promise.resolve(),
    startRuntime,
    stopRuntime() {
      currentControl = stoppedControl;
      currentCaption = stoppedCaption;
      return Promise.resolve(stoppedControl);
    },
    getRuntimeControlSnapshot: () => Promise.resolve(currentControl),
    getCaptionAggregateSnapshot: () => Promise.resolve(currentCaption),
    saveAppConfig: () => Promise.resolve(currentControl),
    listAudioInputDevices: () => Promise.resolve([]),
    probeAudioInput: () =>
      Promise.resolve({
        sampleRate: 48_000,
        durationMs: 2_000,
        rmsDbfs: -24,
        peakDbfs: -6,
        clipping: false,
        gateOpen: true,
      }),
    saveCredential: () => Promise.resolve(currentControl),
    deleteCredential: () => Promise.resolve(currentControl),
  };
  gatewayHarness.current = gateway;

  let runtime!: ReturnType<typeof useRuntime>;
  const app = renderer.createApp(
    defineComponent({
      setup() {
        runtime = useRuntime();
        return () => null;
      },
    }),
  );
  app.mount({ parent: null, text: "", children: [] });

  return {
    app,
    pendingStart,
    restartedCaption,
    restartedControl,
    runtime,
    startRuntime,
    unsubscribeControl,
    unsubscribeEvents,
    emitControlSnapshot(snapshot: RuntimeControlSnapshot) {
      currentControl = snapshot;
      controlListener(snapshot);
    },
    publishEvent(event: Parameters<RuntimeEventListener>[0]) {
      eventListener(event);
    },
    setCaption(snapshot: CaptionAggregateSnapshot) {
      currentCaption = snapshot;
    },
  };
}

test("keeps the public runtime composable surface stable", async () => {
  const harness = mountRuntimeHarness();

  try {
    await vi.waitFor(() => {
      expect(harness.runtime.runtimeStatus.value.status).toBe("running");
    });

    expect(Object.keys(harness.runtime).sort()).toEqual([
      "audioInputDevices",
      "audioProbeFailure",
      "audioProbeResult",
      "captionPreviewStatus",
      "completedCaptions",
      "credentialFailure",
      "credentialStatuses",
      "currentGeneration",
      "currentGenerationCaptionPipelinePlan",
      "currentGenerationSelection",
      "currentGenerationUploadsMicrophoneAudio",
      "deleteCredential",
      "desiredCaptionPipelinePlan",
      "desiredConfig",
      "diagnostics",
      "inFlightRuntimeAction",
      "isAudioProbeRunning",
      "isCredentialBusy",
      "isRuntimeBusy",
      "isSettingsBusy",
      "latestAudioLevel",
      "loadAudioInputDevices",
      "pendingGenerationChanges",
      "probeAudioInput",
      "runAction",
      "runtimeFailure",
      "runtimeStatus",
      "saveConfig",
      "saveCredential",
      "settingsFailure",
      "visibleCaptionText",
    ]);
  } finally {
    harness.app.unmount();
  }
});

test("connects on mount and disposes subscriptions on unmount", async () => {
  const harness = mountRuntimeHarness();

  await vi.waitFor(() => {
    expect(harness.runtime.runtimeStatus.value.status).toBe("running");
  });

  harness.app.unmount();

  expect(harness.unsubscribeEvents).toHaveBeenCalledOnce();
  expect(harness.unsubscribeControl).toHaveBeenCalledOnce();
});

test("routes realtime audio levels outside the lifecycle reducer", async () => {
  const harness = mountRuntimeHarness();

  try {
    await vi.waitFor(() => {
      expect(harness.runtime.runtimeStatus.value.status).toBe("running");
    });
    const level = {
      generation: 7,
      revision: 3,
      rmsDbfs: -28,
      peakDbfs: -5,
      clipping: false,
      gateOpen: true,
      timestampMs: 1_010,
    } as const;

    harness.publishEvent({ type: "audioLevel", payload: level });

    expect(harness.runtime.latestAudioLevel.value).toEqual(level);
    expect(harness.runtime.runtimeStatus.value.status).toBe("running");
  } finally {
    harness.app.unmount();
  }
});

test("reopens caption admission when Running arrives before Start resolves", async () => {
  const harness = mountRuntimeHarness();

  try {
    await harness.runtime.runAction("stop");
    expect(harness.runtime.captionPreviewStatus.value).toBe("waiting");

    const start = harness.runtime.runAction("start");
    await vi.waitFor(() => {
      expect(harness.startRuntime).toHaveBeenCalledOnce();
    });

    harness.setCaption(harness.restartedCaption);
    harness.emitControlSnapshot(harness.restartedControl);
    expect(harness.runtime.runtimeStatus.value.status).toBe("running");

    harness.pendingStart.resolve(harness.restartedControl);
    await start;

    expect(harness.runtime.captionPreviewStatus.value).toBe("listening");
  } finally {
    harness.app.unmount();
  }
});

test("keeps an authoritative Error visible when Start rejects", async () => {
  const harness = mountRuntimeHarness();

  try {
    await harness.runtime.runAction("stop");

    const start = harness.runtime.runAction("start");
    await vi.waitFor(() => {
      expect(harness.startRuntime).toHaveBeenCalledOnce();
    });

    const restartedGeneration = harness.restartedControl.generation;
    if (restartedGeneration === null) {
      throw new Error(
        "The restarted control snapshot must contain a generation.",
      );
    }
    const errorControl: RuntimeControlSnapshot = {
      ...harness.restartedControl,
      runtimeStatus: {
        status: "error",
        message: "Recognition connection failed",
        timestampMs: 850,
      },
      generation: {
        ...restartedGeneration,
        phase: "error",
      },
    };
    harness.emitControlSnapshot(errorControl);
    expect(harness.runtime.runtimeStatus.value.status).toBe("error");

    harness.pendingStart.reject(new Error("Start IPC rejected"));
    await start;

    expect(harness.runtime.runtimeStatus.value).toEqual(
      errorControl.runtimeStatus,
    );
  } finally {
    harness.app.unmount();
  }
});

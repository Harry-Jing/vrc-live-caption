import { createRenderer, defineComponent } from "vue";
import { expect, test, vi } from "vitest";
import captionSessionFixture from "../../contracts/caption-session-snapshot-v1.json?raw";
import runtimeControlFixture from "../../contracts/runtime-control-snapshot-v3.json?raw";
import type {
  RuntimeBackend,
  RuntimeControlListener,
  RuntimeEventListener,
} from "./backend";
import { decodeCaptionSessionSnapshotV1 } from "./wire/captionSessionContract";
import { decodeRuntimeControlSnapshotV3 } from "./wire/runtimeControlContract";
import type { CaptionSessionSnapshotV1, RuntimeControlSnapshot } from "./types";
import { useRuntime } from "./useRuntime";

const backendHarness: { current: RuntimeBackend | undefined } = vi.hoisted(
  () => ({ current: undefined }),
);

vi.mock("../platform/runtimeBackend", () => ({
  createRuntimeBackend: () => backendHarness.current,
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
  const fixtureControl = decodeRuntimeControlSnapshotV3(
    JSON.parse(runtimeControlFixture) as unknown,
  );
  const fixtureCaption = decodeCaptionSessionSnapshotV1(
    JSON.parse(captionSessionFixture) as unknown,
  );
  if (fixtureControl.session === null) {
    throw new Error("The runtime control fixture must contain a session.");
  }
  const fixtureSession = fixtureControl.session;
  const initialControl: RuntimeControlSnapshot = {
    ...fixtureControl,
    revision: 1,
    runtime: { status: "running", timestampMs: 1_000 },
    session: {
      ...fixtureSession,
      generation: 7,
    },
  };
  const stoppedControl: RuntimeControlSnapshot = {
    ...initialControl,
    revision: 2,
    runtime: { status: "stopped", timestampMs: 1_100 },
    session: null,
  };
  const restartedControl: RuntimeControlSnapshot = {
    ...initialControl,
    revision: 3,
    runtime: { status: "running", timestampMs: 900 },
    session: {
      ...fixtureSession,
      generation: 8,
    },
  };
  const initialCaption = {
    ...fixtureCaption,
    snapshotRevision: 1,
    active: { generation: 7, streamId: "recognition-7-1" },
    activeUnits: [{ unitId: "speech-7-1", startedAtMs: 1_000 }],
    captions: [],
  };
  const stoppedCaption = {
    ...initialCaption,
    snapshotRevision: 2,
    active: null,
    activeUnits: [],
  };
  const restartedCaption = {
    ...initialCaption,
    snapshotRevision: 3,
    active: { generation: 8, streamId: "recognition-8-1" },
    activeUnits: [{ unitId: "speech-8-1", startedAtMs: 1_200 }],
  };
  const pendingStart = deferred<RuntimeControlSnapshot>();
  let controlListener: RuntimeControlListener = () => {
    throw new Error("The runtime control listener is not registered.");
  };
  let eventListener: RuntimeEventListener = () => {
    throw new Error("The runtime event listener is not registered.");
  };
  let currentControl = initialControl;
  let currentCaption: CaptionSessionSnapshotV1 = initialCaption;
  const startRuntime = vi.fn(() => pendingStart.promise);
  const backend: RuntimeBackend = {
    listen(listener) {
      eventListener = listener;
      return Promise.resolve(() => undefined);
    },
    listenControl(listener) {
      controlListener = listener;
      return Promise.resolve(() => undefined);
    },
    sendOscTestMessage: () => Promise.resolve(),
    startRuntime,
    stopRuntime() {
      currentControl = stoppedControl;
      currentCaption = stoppedCaption;
      return Promise.resolve(stoppedControl);
    },
    getControlSnapshot: () => Promise.resolve(currentControl),
    getCaptionSessionSnapshot: () => Promise.resolve(currentCaption),
    saveConfig: () => Promise.resolve(currentControl),
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
    saveProviderSecret: () => Promise.resolve(currentControl),
    deleteProviderSecret: () => Promise.resolve(currentControl),
  };
  backendHarness.current = backend;

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
    emitControlSnapshot(snapshot: RuntimeControlSnapshot) {
      currentControl = snapshot;
      controlListener(snapshot);
    },
    publishEvent(event: Parameters<RuntimeEventListener>[0]) {
      eventListener(event);
    },
    setCaption(snapshot: CaptionSessionSnapshotV1) {
      currentCaption = snapshot;
    },
  };
}

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
    await harness.runtime.runCommand("stop_runtime");
    expect(harness.runtime.captionPreviewStatus.value).toBe("waiting");

    const start = harness.runtime.runCommand("start_runtime");
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
    await harness.runtime.runCommand("stop_runtime");

    const start = harness.runtime.runCommand("start_runtime");
    await vi.waitFor(() => {
      expect(harness.startRuntime).toHaveBeenCalledOnce();
    });

    const restartedSession = harness.restartedControl.session;
    if (restartedSession === null) {
      throw new Error("The restarted control snapshot must contain a session.");
    }
    const errorControl: RuntimeControlSnapshot = {
      ...harness.restartedControl,
      runtime: {
        status: "error",
        message: "Recognition connection failed",
        timestampMs: 850,
      },
      session: {
        ...restartedSession,
        phase: "error",
      },
    };
    harness.emitControlSnapshot(errorControl);
    expect(harness.runtime.runtimeStatus.value.status).toBe("error");

    harness.pendingStart.reject(new Error("Start IPC rejected"));
    await start;

    expect(harness.runtime.runtimeStatus.value).toEqual(errorControl.runtime);
  } finally {
    harness.app.unmount();
  }
});

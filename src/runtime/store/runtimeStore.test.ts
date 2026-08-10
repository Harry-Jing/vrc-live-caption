import { afterEach, expect, test, vi } from "vitest";
import captionSessionFixture from "../../../contracts/caption-session-snapshot-v1.json?raw";
import runtimeControlFixture from "../../../contracts/runtime-control-snapshot-v3.json?raw";
import type {
  RuntimeBackend,
  RuntimeControlListener,
  RuntimeEventListener,
} from "../backend";
import type {
  CaptionSessionSnapshotV1,
  RuntimeControlSnapshot,
} from "../types";
import { decodeCaptionSessionSnapshotV1 } from "../wire/captionSessionContract";
import { decodeRuntimeControlSnapshotV3 } from "../wire/runtimeControlContract";
import { createRuntimeStore } from "./runtimeStore";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((complete, fail) => {
    resolve = complete;
    reject = fail;
  });
  return { promise, reject, resolve };
}

function createRuntimeStoreHarness() {
  const fixtureControl = decodeRuntimeControlSnapshotV3(
    JSON.parse(runtimeControlFixture) as unknown,
  );
  const fixtureCaption = decodeCaptionSessionSnapshotV1(
    JSON.parse(captionSessionFixture) as unknown,
  );
  if (fixtureControl.session === null) {
    throw new Error("The runtime control fixture must contain a session.");
  }

  const initialControl: RuntimeControlSnapshot = {
    ...fixtureControl,
    revision: 1,
    runtime: { status: "idle", timestampMs: 100 },
    session: null,
  };
  const runningControl: RuntimeControlSnapshot = {
    ...fixtureControl,
    revision: 2,
    runtime: { status: "running", timestampMs: 200 },
    session: {
      ...fixtureControl.session,
      generation: 8,
      phase: "running",
    },
  };
  const initialCaption: CaptionSessionSnapshotV1 = {
    ...fixtureCaption,
    snapshotRevision: 1,
    active: null,
    activeUnits: [],
    captions: [],
  };
  const runningCaption: CaptionSessionSnapshotV1 = {
    ...initialCaption,
    snapshotRevision: 2,
    active: { generation: 8, streamId: "recognition-8-1" },
  };
  const pendingStart = deferred<RuntimeControlSnapshot>();
  const unsubscribeEvents = vi.fn();
  const unsubscribeControl = vi.fn();
  const callOrder: string[] = [];
  let currentControl = initialControl;
  let currentCaption = initialCaption;
  let eventListener: RuntimeEventListener = () => undefined;
  let controlListener: RuntimeControlListener = () => undefined;

  const startRuntime = vi.fn(() => pendingStart.promise);
  const getControlSnapshot = vi.fn(() => {
    callOrder.push("getControlSnapshot");
    return Promise.resolve(currentControl);
  });
  const backend: RuntimeBackend = {
    listen(listener) {
      callOrder.push("listen");
      eventListener = listener;
      return Promise.resolve(unsubscribeEvents);
    },
    listenControl(listener) {
      callOrder.push("listenControl");
      controlListener = listener;
      return Promise.resolve(unsubscribeControl);
    },
    sendOscTestMessage: () => Promise.resolve(),
    startRuntime,
    stopRuntime: () => Promise.resolve(initialControl),
    getControlSnapshot,
    getCaptionSessionSnapshot() {
      callOrder.push("getCaptionSessionSnapshot");
      return Promise.resolve(currentCaption);
    },
    saveConfig: () => Promise.resolve(currentControl),
    listAudioInputDevices() {
      callOrder.push("listAudioInputDevices");
      return Promise.resolve([
        { id: "usb-headset", name: "USB Headset", isDefault: true },
      ]);
    },
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

  return {
    callOrder,
    controlListener,
    eventListener,
    getControlSnapshot,
    pendingStart,
    runningCaption,
    runningControl,
    startRuntime,
    store: createRuntimeStore(backend),
    unsubscribeControl,
    unsubscribeEvents,
    useRunningSnapshots() {
      currentControl = runningControl;
      currentCaption = runningCaption;
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
});

test("connects listeners before synchronizing state and loading devices", async () => {
  const harness = createRuntimeStoreHarness();

  await harness.store.connect();

  expect(harness.store.runtime.runtimeStatus.value.status).toBe("idle");
  expect(harness.store.runtime.audioInputDevices.value).toEqual([
    { id: "usb-headset", name: "USB Headset", isDefault: true },
  ]);
  expect(harness.callOrder).toEqual([
    "listen",
    "listenControl",
    "getControlSnapshot",
    "getCaptionSessionSnapshot",
    "listAudioInputDevices",
  ]);

  harness.store.dispose();
});

test("disposes both backend subscriptions exactly once", async () => {
  const harness = createRuntimeStoreHarness();
  await harness.store.connect();

  harness.store.dispose();
  harness.store.dispose();

  expect(harness.unsubscribeEvents).toHaveBeenCalledOnce();
  expect(harness.unsubscribeControl).toHaveBeenCalledOnce();
});

test("reconciles an unresolved Start from the authoritative control pull", async () => {
  vi.useFakeTimers();
  const harness = createRuntimeStoreHarness();
  await harness.store.connect();
  harness.useRunningSnapshots();

  const start = harness.store.runtime.runCommand("start_runtime");
  await vi.advanceTimersByTimeAsync(0);
  expect(harness.startRuntime).toHaveBeenCalledOnce();
  expect(harness.store.runtime.runtimeStatus.value.status).toBe("starting");

  await vi.advanceTimersByTimeAsync(500);

  expect(harness.getControlSnapshot).toHaveBeenCalledTimes(2);
  expect(harness.store.runtime.runtimeStatus.value.status).toBe("running");

  harness.pendingStart.resolve(harness.runningControl);
  await start;
  harness.store.dispose();
});

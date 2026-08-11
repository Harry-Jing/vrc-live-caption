import { afterEach, expect, test, vi } from "vitest";
import captionAggregateFixture from "../../../contracts/caption-aggregate-snapshot-v1.json?raw";
import runtimeControlFixture from "../../../contracts/runtime-control-snapshot-v1.json?raw";
import type { CaptionAggregateSnapshot } from "../captionAggregate";
import type {
  AppGateway,
  RuntimeControlSnapshotListener,
  RuntimeEventListener,
} from "../gateway";
import type { RuntimeControlSnapshot } from "../runtimeControl";
import { decodeCaptionAggregateSnapshot } from "../wire/captionAggregateContract";
import { decodeRuntimeControlSnapshot } from "../wire/runtimeControlContract";
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
  const fixtureControl = decodeRuntimeControlSnapshot(
    JSON.parse(runtimeControlFixture) as unknown,
  );
  const fixtureCaption = decodeCaptionAggregateSnapshot(
    JSON.parse(captionAggregateFixture) as unknown,
  );
  if (fixtureControl.generation === null) {
    throw new Error("The runtime control fixture must contain a generation.");
  }

  const initialControl: RuntimeControlSnapshot = {
    ...fixtureControl,
    revision: 1,
    runtimeStatus: { status: "idle", timestampMs: 100 },
    generation: null,
  };
  const runningControl: RuntimeControlSnapshot = {
    ...fixtureControl,
    revision: 2,
    runtimeStatus: { status: "running", timestampMs: 200 },
    generation: {
      ...fixtureControl.generation,
      id: 8,
      phase: "running",
    },
  };
  const initialCaption: CaptionAggregateSnapshot = {
    ...fixtureCaption,
    snapshotRevision: 1,
    activeStream: null,
    openSourceUnits: [],
    captions: [],
  };
  const runningCaption: CaptionAggregateSnapshot = {
    ...initialCaption,
    snapshotRevision: 2,
    activeStream: { generation: 8, streamId: "recognition-8-1" },
  };
  const pendingStart = deferred<RuntimeControlSnapshot>();
  const unsubscribeEvents = vi.fn();
  const unsubscribeControl = vi.fn();
  const callOrder: string[] = [];
  let currentControl = initialControl;
  let currentCaption = initialCaption;
  let eventListener: RuntimeEventListener = () => undefined;
  let controlListener: RuntimeControlSnapshotListener = () => undefined;

  const startRuntime = vi.fn(() => pendingStart.promise);
  const sendOscTestMessage = vi.fn(() => Promise.resolve());
  const getRuntimeControlSnapshot = vi.fn(() => {
    callOrder.push("getRuntimeControlSnapshot");
    return Promise.resolve(currentControl);
  });
  const gateway: AppGateway = {
    subscribeRuntimeEvents(listener) {
      callOrder.push("subscribeRuntimeEvents");
      eventListener = listener;
      return Promise.resolve(unsubscribeEvents);
    },
    subscribeRuntimeControlSnapshots(listener) {
      callOrder.push("subscribeRuntimeControlSnapshots");
      controlListener = listener;
      return Promise.resolve(unsubscribeControl);
    },
    sendOscTestMessage,
    startRuntime,
    stopRuntime: () => Promise.resolve(initialControl),
    getRuntimeControlSnapshot,
    getCaptionAggregateSnapshot() {
      callOrder.push("getCaptionAggregateSnapshot");
      return Promise.resolve(currentCaption);
    },
    saveAppConfig: () => Promise.resolve(currentControl),
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
    saveCredential: () => Promise.resolve(currentControl),
    deleteCredential: () => Promise.resolve(currentControl),
  };

  return {
    callOrder,
    controlListener,
    eventListener,
    getRuntimeControlSnapshot,
    pendingStart,
    runningCaption,
    runningControl,
    startRuntime,
    sendOscTestMessage,
    store: createRuntimeStore(gateway),
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
    "subscribeRuntimeEvents",
    "subscribeRuntimeControlSnapshots",
    "getRuntimeControlSnapshot",
    "getCaptionAggregateSnapshot",
    "listAudioInputDevices",
  ]);

  harness.store.dispose();
});

test("disposes both gateway subscriptions exactly once", async () => {
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

  const start = harness.store.runtime.runAction("start");
  await vi.advanceTimersByTimeAsync(0);
  expect(harness.startRuntime).toHaveBeenCalledOnce();
  expect(harness.store.runtime.runtimeStatus.value.status).toBe("starting");

  await vi.advanceTimersByTimeAsync(500);

  expect(harness.getRuntimeControlSnapshot).toHaveBeenCalledTimes(2);
  expect(harness.store.runtime.runtimeStatus.value.status).toBe("running");

  harness.pendingStart.resolve(harness.runningControl);
  await start;
  harness.store.dispose();
});

test("retains a structured runtime action failure", async () => {
  const harness = createRuntimeStoreHarness();
  await harness.store.connect();
  harness.sendOscTestMessage.mockRejectedValueOnce(
    Object.assign(new Error("Chatbox send failed."), {
      code: "osc.send_failed",
    }),
  );

  await harness.store.runtime.runAction("testChatbox");

  expect(harness.store.runtime.runtimeFailure.value).toEqual({
    code: "osc.send_failed",
    message: "Chatbox send failed.",
  });
  harness.store.dispose();
});

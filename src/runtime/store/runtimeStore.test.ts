import { afterEach, expect, test, vi } from "vitest";
import captionAggregateFixture from "../../../contracts/caption-aggregate-snapshot-v2.json?raw";
import runtimeControlFixture from "../../../contracts/runtime-control-snapshot-v3.json?raw";
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
    translationUnits: [],
  };
  const runningCaption: CaptionAggregateSnapshot = {
    ...initialCaption,
    snapshotRevision: 2,
    activeStream: { generation: 8, streamId: "recognition-8-1" },
  };
  const pendingStart = deferred<RuntimeControlSnapshot>();
  const pendingStop = deferred<RuntimeControlSnapshot>();
  const unsubscribeEvents = vi.fn();
  const unsubscribeControl = vi.fn();
  const callOrder: string[] = [];
  let currentControl = initialControl;
  let currentCaption = initialCaption;
  let eventListener: RuntimeEventListener = () => undefined;
  let controlListener: RuntimeControlSnapshotListener = () => undefined;

  const startRuntime = vi.fn(() => pendingStart.promise);
  const stopRuntime = vi.fn(() => pendingStop.promise);
  const sendOscTestMessage = vi.fn(() => Promise.resolve());
  const saveCredential = vi.fn<AppGateway["saveCredential"]>(() =>
    Promise.resolve(currentControl),
  );
  const deleteCredential = vi.fn<AppGateway["deleteCredential"]>(() =>
    Promise.resolve(currentControl),
  );
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
    stopRuntime,
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
    saveCredential,
    deleteCredential,
  };

  return {
    callOrder,
    controlListener,
    deleteCredential,
    eventListener,
    getRuntimeControlSnapshot,
    fixtureCaption,
    initialControl,
    pendingStart,
    pendingStop,
    runningCaption,
    runningControl,
    saveCredential,
    startRuntime,
    stopRuntime,
    sendOscTestMessage,
    store: createRuntimeStore(gateway),
    unsubscribeControl,
    unsubscribeEvents,
    useRunningSnapshots() {
      currentControl = runningControl;
      currentCaption = runningCaption;
    },
    useSnapshots(
      control: RuntimeControlSnapshot,
      captions: CaptionAggregateSnapshot,
    ) {
      currentControl = control;
      currentCaption = captions;
    },
  };
}

function useTranslationSnapshots(
  harness: ReturnType<typeof createRuntimeStoreHarness>,
) {
  const generation = harness.runningControl.generation;
  const translation = harness.runningControl.desired.config.translation;
  if (generation === null || translation === null) {
    throw new Error("Translation fixtures require a generation and selection.");
  }

  harness.useSnapshots(
    {
      ...harness.runningControl,
      generation: {
        ...generation,
        id: 7,
        selection: {
          ...generation.selection,
          publication: { mode: "completed", content: "bilingual" },
          translation,
        },
        translationState: { state: "active" },
        uploadsSourceText: true,
      },
    },
    harness.fixtureCaption,
  );
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

test("attributes concurrent operation state to each credential", async () => {
  const harness = createRuntimeStoreHarness();
  const openAiSave = deferred<RuntimeControlSnapshot>();
  const customSave = deferred<RuntimeControlSnapshot>();
  harness.saveCredential.mockImplementation((id) =>
    id === "openai" ? openAiSave.promise : customSave.promise,
  );

  const saveOpenAi = harness.store.runtime.saveCredential(
    "openai",
    "test-openai-secret",
  );
  const saveCustom = harness.store.runtime.saveCredential(
    "customTranslation",
    "test-custom-secret",
  );

  expect(harness.store.runtime.credentialOperationStates.value).toEqual({
    openai: { failure: null, isBusy: true },
    customTranslation: { failure: null, isBusy: true },
  });

  customSave.reject(
    Object.assign(new Error("Custom credential save failed."), {
      code: "credential.custom_save_failed",
    }),
  );
  await saveCustom;

  expect(
    harness.store.runtime.credentialOperationStates.value.customTranslation,
  ).toEqual({
    failure: {
      code: "credential.custom_save_failed",
      message: "Custom credential save failed.",
    },
    isBusy: false,
  });
  expect(harness.store.runtime.credentialOperationStates.value.openai).toEqual({
    failure: null,
    isBusy: true,
  });
  expect(harness.store.runtime.credentialFailure.value).toBeNull();
  expect(harness.store.runtime.isCredentialBusy.value).toBe(true);

  openAiSave.resolve(harness.runningControl);
  await saveOpenAi;

  harness.deleteCredential.mockResolvedValueOnce(harness.runningControl);
  const deleteCustom =
    harness.store.runtime.deleteCredential("customTranslation");
  expect(
    harness.store.runtime.credentialOperationStates.value.customTranslation,
  ).toEqual({ failure: null, isBusy: true });

  await deleteCustom;
  expect(
    harness.store.runtime.credentialOperationStates.value.customTranslation,
  ).toEqual({ failure: null, isBusy: false });
  harness.store.dispose();
});

test("lets the newest same-credential request own failure feedback", async () => {
  const harness = createRuntimeStoreHarness();
  const olderSave = deferred<RuntimeControlSnapshot>();
  const newerSave = deferred<RuntimeControlSnapshot>();
  harness.saveCredential
    .mockImplementationOnce(() => olderSave.promise)
    .mockImplementationOnce(() => newerSave.promise);

  const olderResult = harness.store.runtime.saveCredential(
    "openai",
    "test-older-secret",
  );
  const newerResult = harness.store.runtime.saveCredential(
    "openai",
    "test-newer-secret",
  );

  newerSave.resolve(harness.runningControl);
  await newerResult;
  expect(harness.store.runtime.credentialOperationStates.value.openai).toEqual({
    failure: null,
    isBusy: true,
  });

  olderSave.reject(new Error("Stale credential failure."));
  await olderResult;
  expect(harness.store.runtime.credentialOperationStates.value.openai).toEqual({
    failure: null,
    isBusy: false,
  });
  harness.store.dispose();
});

test("reconstructs the same Translation view from authoritative pulls", async () => {
  const first = createRuntimeStoreHarness();
  const reconnected = createRuntimeStoreHarness();
  useTranslationSnapshots(first);
  useTranslationSnapshots(reconnected);

  await first.store.connect();
  await reconnected.store.connect();

  expect(reconnected.store.runtime.translationPresentation.value).toEqual(
    first.store.runtime.translationPresentation.value,
  );
  expect(
    reconnected.store.runtime.translationPresentation.value.units,
  ).toHaveLength(3);

  first.store.dispose();
  reconnected.store.dispose();
});

test("hides admitted Translation units immediately when Stop is requested", async () => {
  const harness = createRuntimeStoreHarness();
  useTranslationSnapshots(harness);
  await harness.store.connect();
  expect(
    harness.store.runtime.translationPresentation.value.units,
  ).toHaveLength(3);

  const stop = harness.store.runtime.runAction("stop");
  await Promise.resolve();

  expect(harness.store.runtime.translationPresentation.value.units).toEqual([]);

  harness.pendingStop.resolve({
    ...harness.initialControl,
    revision: 3,
    runtimeStatus: { status: "stopped", timestampMs: 300 },
    desired: harness.runningControl.desired,
  });
  await stop;

  expect(harness.store.runtime.translationPresentation.value).toMatchObject({
    state: "inactive",
    units: [],
  });
  harness.store.dispose();
});

import { expect, test } from "vitest";
import captionSessionFixture from "../../contracts/caption-session-snapshot-v1.json?raw";
import { createPreviewBackend } from "./previewBackend";
import { createTauriBackend, type TauriBackendBridge } from "./tauriBackend";
import type { CaptionSessionSnapshotV1, RuntimeEvent } from "./types";

test("TauriBackend decodes the authoritative caption-session pull", async () => {
  const payload = JSON.parse(captionSessionFixture) as unknown;
  const bridge: TauriBackendBridge = {
    listen() {
      return Promise.resolve(() => undefined);
    },
    invoke<Result>(command: string) {
      if (command !== "get_caption_session_snapshot") {
        return Promise.reject(new Error(`Unexpected command: ${command}`));
      }

      return Promise.resolve(payload as Result);
    },
  };
  const backend = createTauriBackend(bridge);

  await expect(backend.getCaptionSessionSnapshot()).resolves.toMatchObject({
    contractVersion: 1,
    snapshotRevision: 3,
    active: { generation: 7, streamId: "recognition-7-1" },
  });
});

test("TauriBackend decodes caption-session-changed before delivery", async () => {
  const payload = JSON.parse(captionSessionFixture) as unknown;
  let deliverCaption:
    ((event: Readonly<{ payload: unknown }>) => void) | undefined;
  const bridge: TauriBackendBridge = {
    listen(eventName, listener) {
      if (eventName === "caption-session-changed") {
        deliverCaption = listener;
      }

      return Promise.resolve(() => undefined);
    },
    invoke<Result>() {
      return Promise.resolve(undefined as Result);
    },
  };
  const backend = createTauriBackend(bridge);
  const received: unknown[] = [];
  const unsubscribe = await backend.listen((event) => {
    if (event.type === "captionSessionChanged") {
      received.push(event.payload);
    }
  });

  deliverCaption?.({ payload });
  unsubscribe();

  expect(received).toHaveLength(1);
  expect(received[0]).toMatchObject({
    contractVersion: 1,
    snapshotRevision: 3,
  });
});

test("PreviewBackend publishes and pulls the same full mock aggregates", async () => {
  const backend = createPreviewBackend();
  const aggregates: unknown[] = [];
  const unsubscribe = await backend.listen((event) => {
    if (event.type === "captionSessionChanged") {
      aggregates.push(event.payload);
    }
  });
  const initialControl = await backend.getControlSnapshot();
  await backend.saveConfig({
    ...initialControl.desired.config,
    stt: {
      ...initialControl.desired.config.stt,
      provider: "mock",
      model: "mock",
    },
  });
  await backend.startRuntime();
  await backend.runCommand("emit_mock_transcript");
  const pulled = await backend.getCaptionSessionSnapshot();
  unsubscribe();

  const ongoing = aggregates.find(
    (candidate) =>
      typeof candidate === "object" &&
      candidate !== null &&
      "captions" in candidate &&
      Array.isArray(candidate.captions) &&
      candidate.captions.some(
        (caption) =>
          typeof caption === "object" &&
          caption !== null &&
          "state" in caption &&
          caption.state === "ongoing",
      ),
  );

  expect(ongoing).toBeDefined();
  expect(pulled).toEqual(aggregates.at(-1));
  expect(pulled.activeUnits).toEqual([]);
  expect(pulled.captions[0]).toMatchObject({
    generation: pulled.active?.generation,
    streamId: pulled.active?.streamId,
    lane: "source",
    revision: 2,
    state: "completed",
    provider: "mock",
    model: "mock",
  });
});

test("PreviewBackend mock-bounded emits one unitful completed revision", async () => {
  const backend = createPreviewBackend();
  const initial = await backend.getControlSnapshot();
  await backend.saveConfig({
    ...initial.desired.config,
    stt: {
      ...initial.desired.config.stt,
      provider: "mock",
      model: "mock-bounded",
    },
    publication: { mode: "completed" },
  });
  await backend.startRuntime();
  await backend.runCommand("emit_mock_transcript");

  const pulled = await backend.getCaptionSessionSnapshot();
  const caption = pulled.captions[0];

  expect(pulled.activeUnits).toEqual([]);
  expect(pulled.captions).toHaveLength(1);
  expect(caption).toMatchObject({
    lane: "source",
    revision: 1,
    state: "completed",
    provider: "mock",
    model: "mock-bounded",
  });
  expect(caption?.unitId).not.toBeNull();
  expect(caption?.unitStartedAtMs).not.toBeNull();
});

test("PreviewBackend mock-ongoing-completed supports Live with full unit revisions", async () => {
  const backend = createPreviewBackend();
  const initial = await backend.getControlSnapshot();
  await backend.saveConfig({
    ...initial.desired.config,
    stt: {
      ...initial.desired.config.stt,
      provider: "mock",
      model: "mock-ongoing-completed",
    },
    publication: { mode: "live" },
  });
  const aggregates: CaptionSessionSnapshotV1[] = [];
  const unsubscribe = await backend.listen((event) => {
    if (event.type === "captionSessionChanged") {
      aggregates.push(event.payload);
    }
  });

  try {
    await backend.startRuntime();
    await backend.runCommand("emit_mock_transcript");
  } finally {
    unsubscribe();
  }

  const revisions = aggregates
    .flatMap((aggregate) => aggregate.captions)
    .filter((caption) => caption.model === "mock-ongoing-completed");
  expect(revisions).toEqual([
    expect.objectContaining({
      revision: 1,
      text: "Testing live caption preview...",
      state: "ongoing",
    }),
    expect.objectContaining({
      revision: 2,
      text: "Testing live caption preview from the mock runtime.",
      state: "completed",
    }),
  ]);
  expect(revisions.every((caption) => caption.unitId !== null)).toBe(true);
});

test("PreviewBackend mock-ongoing-only stays unitless and never completes", async () => {
  const backend = createPreviewBackend();
  const initial = await backend.getControlSnapshot();
  await backend.saveConfig({
    ...initial.desired.config,
    stt: {
      ...initial.desired.config.stt,
      provider: "mock",
      model: "mock-ongoing-only",
    },
    publication: { mode: "live" },
  });
  const events: RuntimeEvent[] = [];
  const unsubscribe = await backend.listen((event) => {
    events.push(event);
  });

  try {
    await backend.startRuntime();
    await backend.runCommand("emit_mock_transcript");
    await backend.runCommand("emit_mock_transcript");
  } finally {
    unsubscribe();
  }

  const revisions = events
    .filter((event) => event.type === "captionSessionChanged")
    .flatMap((event) => event.payload.captions)
    .filter((caption) => caption.model === "mock-ongoing-only");
  expect(revisions.map((caption) => caption.revision)).toEqual([1, 2, 3, 4]);
  expect(revisions.map((caption) => caption.text)).toEqual([
    "Testing live caption preview...",
    "Testing live caption preview from the ongoing-only mock runtime.",
    "Testing live caption preview...",
    "Testing live caption preview from the ongoing-only mock runtime.",
  ]);
  expect(
    revisions.every(
      (caption) =>
        caption.unitId === null &&
        caption.unitStartedAtMs === null &&
        caption.state === "ongoing",
    ),
  ).toBe(true);
  expect(events.some((event) => event.type === "utteranceStarted")).toBe(false);

  await backend.stopRuntime();
  expect(await backend.getCaptionSessionSnapshot()).toMatchObject({
    active: null,
    activeUnits: [],
    captions: [],
  });
});

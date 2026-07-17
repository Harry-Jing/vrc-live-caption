import { expect, test } from "vitest";
import captionSessionFixture from "../../contracts/caption-session-snapshot-v1.json?raw";
import { createPreviewBackend } from "./previewBackend";
import { createTauriBackend, type TauriBackendBridge } from "./tauriBackend";

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

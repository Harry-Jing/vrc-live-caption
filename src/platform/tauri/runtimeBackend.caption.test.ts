import { expect, test } from "vitest";
import captionSessionFixture from "../../../contracts/caption-session-snapshot-v1.json?raw";
import { createTauriBackend, type TauriBackendBridge } from "./runtimeBackend";

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

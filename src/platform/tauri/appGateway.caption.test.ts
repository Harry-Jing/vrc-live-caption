import { expect, test } from "vitest";
import captionAggregateFixture from "../../../contracts/caption-aggregate-snapshot-v2.json?raw";
import { RUNTIME_EVENTS, TAURI_COMMANDS } from "../../runtime/wire/tauriIpc";
import { createTauriAppGateway, type TauriIpcBridge } from "./appGateway";

test("Tauri AppGateway decodes the authoritative caption aggregate pull", async () => {
  const payload = JSON.parse(captionAggregateFixture) as unknown;
  const bridge: TauriIpcBridge = {
    listen() {
      return Promise.resolve(() => undefined);
    },
    invoke<Result>(command: string) {
      if (command !== TAURI_COMMANDS.getCaptionAggregateSnapshot) {
        return Promise.reject(new Error(`Unexpected command: ${command}`));
      }

      return Promise.resolve(payload as Result);
    },
  };
  const gateway = createTauriAppGateway(bridge);

  await expect(gateway.getCaptionAggregateSnapshot()).resolves.toMatchObject({
    contractVersion: 2,
    snapshotRevision: 9,
    activeStream: { generation: 7, streamId: "recognition-7-1" },
  });
});

test("Tauri AppGateway decodes caption-aggregate-changed before delivery", async () => {
  const payload = JSON.parse(captionAggregateFixture) as unknown;
  let deliverCaption:
    ((event: Readonly<{ payload: unknown }>) => void) | undefined;
  const bridge: TauriIpcBridge = {
    listen(eventName, listener) {
      if (eventName === RUNTIME_EVENTS.captionAggregateChanged) {
        deliverCaption = listener;
      }

      return Promise.resolve(() => undefined);
    },
    invoke<Result>() {
      return Promise.resolve(undefined as Result);
    },
  };
  const gateway = createTauriAppGateway(bridge);
  const received: unknown[] = [];
  const unsubscribe = await gateway.subscribeRuntimeEvents((event) => {
    if (event.type === "captionAggregateChanged") {
      received.push(event.payload);
    }
  });

  deliverCaption?.({ payload });
  unsubscribe();

  expect(received).toHaveLength(1);
  expect(received[0]).toMatchObject({
    contractVersion: 2,
    snapshotRevision: 9,
  });
});

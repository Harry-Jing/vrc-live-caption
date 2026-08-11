import { expect, test } from "vitest";
import { TAURI_COMMANDS } from "../../runtime/wire/tauriIpc";
import { createTauriAppGateway, type TauriIpcBridge } from "./appGateway";

test("Tauri AppGateway normalizes a structured AppError rejection", async () => {
  const bridge: TauriIpcBridge = {
    listen() {
      return Promise.resolve(() => undefined);
    },
    invoke(command) {
      if (command !== TAURI_COMMANDS.sendOscTestMessage) {
        return Promise.reject(new Error(`Unexpected command: ${command}`));
      }

      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- Tauri transports serialized AppError objects, not JavaScript Error instances.
      return Promise.reject({
        code: "osc.send_failed",
        message: "Chatbox send failed.",
      });
    },
  };
  const gateway = createTauriAppGateway(bridge);

  await expect(gateway.sendOscTestMessage()).rejects.toMatchObject({
    name: "TauriAppError",
    code: "osc.send_failed",
    message: "Chatbox send failed.",
  });
});

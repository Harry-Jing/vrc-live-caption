import { expect, test } from "vitest";
import tauriIpcFixture from "../../contracts/tauri-ipc-v1.json?raw";
import { TAURI_COMMANDS } from "./tauriBackend";
import { RUNTIME_CONTROL_EVENT, RUNTIME_EVENTS } from "./types";

test("Tauri event and command names match the shared IPC manifest", () => {
  expect(JSON.parse(tauriIpcFixture) as unknown).toEqual({
    events: {
      runtimeStatus: RUNTIME_EVENTS.status,
      runtimeControlChanged: RUNTIME_CONTROL_EVENT,
      captionSessionChanged: RUNTIME_EVENTS.captionSessionChanged,
      audioLevel: RUNTIME_EVENTS.audioLevel,
      diagnostic: RUNTIME_EVENTS.diagnostic,
    },
    commands: TAURI_COMMANDS,
  });
});

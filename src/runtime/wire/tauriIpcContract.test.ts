import { expect, test } from "vitest";
import tauriIpcFixture from "../../../contracts/tauri-ipc-v2.json?raw";
import {
  RUNTIME_CONTROL_EVENT,
  RUNTIME_EVENTS,
  TAURI_COMMANDS,
} from "./tauriIpc";

test("Tauri event and command names match the shared IPC manifest", () => {
  const manifest = JSON.parse(tauriIpcFixture) as {
    events: Record<string, string>;
    commands: typeof TAURI_COMMANDS;
  };

  expect(manifest).toEqual({
    events: {
      runtimeStatus: RUNTIME_EVENTS.status,
      runtimeControlChanged: RUNTIME_CONTROL_EVENT,
      captionAggregateChanged: RUNTIME_EVENTS.captionAggregateChanged,
      audioLevel: RUNTIME_EVENTS.audioLevel,
      diagnostic: RUNTIME_EVENTS.diagnostic,
    },
    commands: TAURI_COMMANDS,
  });

  const declaredUiEventNames = [
    ...Object.values(RUNTIME_EVENTS),
    RUNTIME_CONTROL_EVENT,
  ].sort();
  expect(Object.values(manifest.events).sort()).toEqual(declaredUiEventNames);
});

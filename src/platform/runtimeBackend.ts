// Backend gateway for the runtime UI. The interface is implemented once per
// environment (Tauri IPC, browser-only preview, unsupported) and selected a
// single time at startup, so composables and components never branch on the
// execution environment themselves.

import { isTauri } from "@tauri-apps/api/core";
import { uiText } from "../i18n/uiText";
import { createPreviewBackend } from "../preview/runtimeBackend";
import type { RuntimeBackend } from "../runtime/backend";
import { createTauriBackend } from "./tauri/runtimeBackend";

function createUnsupportedBackend(): RuntimeBackend {
  const reject = () =>
    Promise.reject(new Error(uiText("runtime.errors.desktopRequired")));

  return {
    listen: reject,
    listenControl: reject,
    sendOscTestMessage: reject,
    startRuntime: reject,
    stopRuntime: reject,
    getControlSnapshot: reject,
    getCaptionSessionSnapshot: reject,
    saveConfig: reject,
    listAudioInputDevices: reject,
    probeAudioInput: reject,
    saveProviderSecret: reject,
    deleteProviderSecret: reject,
  };
}

export function createRuntimeBackend(): RuntimeBackend {
  if (isTauri()) {
    return createTauriBackend();
  }

  if (import.meta.env.DEV) {
    return createPreviewBackend();
  }

  return createUnsupportedBackend();
}

// Host gateway for the App. The interface is implemented once per
// environment (Tauri IPC, browser-only preview, unsupported) and selected a
// single time at startup, so composables and components never branch on the
// execution environment themselves.

import { isTauri } from "@tauri-apps/api/core";
import { uiText } from "../i18n/uiText";
import type { AppGateway } from "../runtime/gateway";
import { createPreviewAppGateway } from "./preview/appGateway";
import { createTauriAppGateway } from "./tauri/appGateway";

function createUnsupportedAppGateway(): AppGateway {
  const reject = () =>
    Promise.reject(new Error(uiText("runtime.errors.desktopRequired")));

  return {
    subscribeRuntimeEvents: reject,
    subscribeRuntimeControlSnapshots: reject,
    sendOscTestMessage: reject,
    startRuntime: reject,
    stopRuntime: reject,
    getRuntimeControlSnapshot: reject,
    getCaptionAggregateSnapshot: reject,
    saveAppConfig: reject,
    listAudioInputDevices: reject,
    probeAudioInput: reject,
    saveCredential: reject,
    deleteCredential: reject,
  };
}

export function createAppGateway(): AppGateway {
  if (isTauri()) {
    return createTauriAppGateway();
  }

  if (import.meta.env.DEV) {
    return createPreviewAppGateway(
      typeof location === "undefined" ? "" : location.search,
    );
  }

  return createUnsupportedAppGateway();
}

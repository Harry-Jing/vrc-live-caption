export const RUNTIME_EVENTS = {
  status: "runtime-status",
  audioLevel: "audio-level",
  captionAggregateChanged: "caption-aggregate-changed",
  diagnostic: "diagnostic-event",
} as const;

export const RUNTIME_CONTROL_EVENT = "runtime-control-changed" as const;

export const TAURI_COMMANDS = {
  saveAppConfig: "save_app_config",
  listAudioInputDevices: "list_audio_input_devices",
  probeAudioInput: "probe_audio_input",
  startRuntime: "start_runtime",
  stopRuntime: "stop_runtime",
  getRuntimeControlSnapshot: "get_runtime_control_snapshot",
  getCaptionAggregateSnapshot: "get_caption_aggregate_snapshot",
  sendOscTestMessage: "send_osc_test_message",
  saveCredential: "save_credential",
  deleteCredential: "delete_credential",
} as const;

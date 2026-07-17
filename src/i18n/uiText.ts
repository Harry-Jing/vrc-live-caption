const englishMessages = {
  "app.name": "VRC Live Caption",
  "app.mode.outgoingCaption": "Outgoing Caption",

  "common.loading": "loading",

  "navigation.primary": "Primary",
  "navigation.live": "Live",
  "navigation.settings": "Settings",
  "navigation.diagnostics": "Diagnostics",

  "audio.devices.default": "Default device",
  "audio.devices.defaultInput": "Default input device",
  "audio.devices.defaultNamed": ({ name }: { name: string }) =>
    `${name} (default)`,
  "audio.devices.savedDisconnected": "Saved device (not connected)",

  "stt.providers.openai": "OpenAI",
  "stt.providers.mock": "Mock",

  "runtime.title": "Runtime",
  "runtime.status.idle": "Idle",
  "runtime.status.starting": "Starting",
  "runtime.status.running": "Running",
  "runtime.status.stopping": "Stopping",
  "runtime.status.stopped": "Stopped",
  "runtime.status.error": "Error",
  "runtime.status.initialIdleMessage": "Runtime is idle",
  "runtime.status.noMessage": "No runtime status message.",
  "runtime.actions.start": "Start",
  "runtime.actions.stop": "Stop",
  "runtime.actions.mockTranscript": "Mock Transcript",
  "runtime.actions.oscTest": "OSC Test",
  "runtime.errors.actionFailed": "Runtime action failed",
  "runtime.errors.unknownAction": "Action failed.",

  "caption.preview.eyebrow": "Caption Preview",
  "caption.preview.title": "Current output",
  "caption.mode.waiting": "Waiting",
  "caption.mode.listening": "Listening",
  "caption.mode.partial": "Partial",
  "caption.mode.final": "Final",
  "caption.state.waiting": "Waiting for transcript events.",
  "caption.finalAnnouncement": ({ text }: { text: string }) =>
    `Final caption: ${text}`,

  "diagnostics.title": "Diagnostics",
  "diagnostics.page.title": "Runtime events and transcripts",
  "diagnostics.empty": "No diagnostics yet.",
  "diagnostics.finalTranscripts.title": "Final Transcripts",
  "diagnostics.finalTranscripts.empty": "No final transcript events yet.",
  "diagnostics.severity.info": "Info",
  "diagnostics.severity.warning": "Warning",
  "diagnostics.severity.error": "Error",
  "diagnostics.category.config": "Config",
  "diagnostics.category.runtime": "Runtime",
  "diagnostics.category.audio": "Audio",
  "diagnostics.category.stt": "STT",
  "diagnostics.category.osc": "OSC",

  "live.eyebrow": "Live caption",
  "live.title": "Speak, preview, send final text",
  "live.chatbox.on": "Chatbox on",
  "live.chatbox.off": "Chatbox off",
  "live.chatbox.unavailable": "Chatbox unavailable",
  "live.currentSetup.title": "Current setup",
  "live.currentSetup.activeSessionTitle": "Active session setup",
  "live.currentSetup.failedSessionTitle": "Failed session setup",
  "live.currentSetup.nextStartTitle": "Next Start setup",
  "live.currentSetup.nextStartBadge": "Next Start",
  "live.currentSetup.pendingChanges.title": "Saved changes are pending",
  "live.currentSetup.pendingChanges.description":
    "The active session is unchanged. Saved settings will apply after Stop and the next Start.",
  "live.currentSetup.pendingChanges.failedDescription":
    "The failed session is retained for diagnostics. Saved settings will be used on the next Start.",
  "live.currentSetup.edit": "Edit",
  "live.currentSetup.microphone": "Microphone",
  "live.currentSetup.sttModel": "STT model",
  "live.currentSetup.oscTarget": "OSC / Test target",
  "live.recentActivity.title": "Recent activity",
  "live.recentActivity.open": "Open",
  "live.recentActivity.latestFinalTranscript": "Latest final transcript",
  "live.recentActivity.noFinalTranscript": "No final transcript yet.",
  "live.recentActivity.latestDiagnostic": "Latest diagnostic",

  "settings.title": "Settings",
  "settings.page.title": "Capture, provider, and output",
  "settings.description":
    "Configure capture, provider credentials, and Chatbox output.",
  "settings.actions.refreshDevices": "Refresh devices",
  "settings.actions.save": "Save Settings",
  "settings.errors.actionFailed": "Settings action failed",
  "settings.feedback.saved": "Settings saved",
  "settings.feedback.nextStart.title": "Saved for the next Start",
  "settings.feedback.nextStart.description": ({
    changes,
  }: {
    changes: string;
  }) =>
    `The active session is unchanged. Saved changes to ${changes} will take effect after Stop and the next Start.`,
  "settings.feedback.nextStart.failedDescription": ({
    changes,
  }: {
    changes: string;
  }) =>
    `The failed session is retained for diagnostics. Saved changes to ${changes} will be used on the next Start.`,
  "settings.feedback.nextStart.change.microphone": "microphone",
  "settings.feedback.nextStart.change.recognition": "speech recognition",
  "settings.feedback.nextStart.change.credential": "provider credentials",
  "settings.feedback.nextStart.change.chatboxOutput": "Chatbox output",
  "settings.feedback.nextStart.change.publication": "publication timing",
  "settings.sections.audio": "Audio",
  "settings.sections.speechProvider": "Speech provider",
  "settings.sections.chatboxOutput": "Chatbox output",
  "settings.fields.microphone": "Microphone",
  "settings.fields.provider": "Provider",
  "settings.fields.language": "Language",
  "settings.fields.sttModel": "STT model",
  "settings.fields.oscHost": "OSC host",
  "settings.fields.port": "Port",
  "settings.fields.chatboxOutput": "Chatbox output",
  "settings.fields.partialPreview": "App partial preview",
  "settings.loading": "Loading settings...",
  "settings.loadFailed": "Settings could not be loaded.",
  "settings.credentials.openai.title": "OpenAI credentials",
  "settings.credentials.openai.cloudDisclosure":
    "When OpenAI cloud speech recognition is selected, microphone audio is uploaded to OpenAI for transcription.",
  "settings.credentials.openai.activeCloudSession.title":
    "The active session still uses OpenAI",
  "settings.credentials.openai.activeCloudSession.description":
    "The provider selection shown in this form does not change the active session. Microphone audio will continue to be uploaded to OpenAI until you Stop the runtime.",
  "settings.credentials.openai.apiKey": "API key",
  "settings.credentials.openai.apiKeyPlaceholder": "sk-...",
  "settings.credentials.openai.actions.save": "Save Key",
  "settings.credentials.openai.actions.remove": "Remove key",
  "settings.credentials.openai.errors.actionFailed": "API key action failed",
  "settings.credentials.openai.status.checking": "Checking",
  "settings.credentials.openai.status.notSaved": "Not saved",
  "settings.credentials.openai.status.environment": ({
    displaySuffix,
  }: {
    displaySuffix: string | null;
  }) => `Env ${displaySuffix ? `...${displaySuffix}` : "saved"}`,
  "settings.credentials.openai.status.system": ({
    displaySuffix,
  }: {
    displaySuffix: string | null;
  }) => `System ${displaySuffix ? `...${displaySuffix}` : "saved"}`,
  "settings.credentials.openai.removeDialog.title": "Remove OpenAI API key?",
  "settings.credentials.openai.removeDialog.description":
    "The saved key will be removed from the system credential store. You can add it again later.",
  "settings.credentials.openai.removeDialog.activeSessionDescription":
    "The saved key will be removed from the system credential store. The active session keeps the credential captured at Start until you Stop the runtime.",
  "settings.credentials.openai.removeDialog.cancel": "Cancel",
  "settings.credentials.openai.removeDialog.confirm": "Remove API key",

  "notFound.eyebrow": "Page not found",
  "notFound.title": "This route does not exist",
  "notFound.backToLive": "Back to Live",
} as const;

type EnglishMessages = typeof englishMessages;

export type UiMessageKey = keyof typeof englishMessages;

type UiMessageFormatter = (...parameters: never[]) => string;

export type UiFormattedMessageKey = {
  [Key in UiMessageKey]: EnglishMessages[Key] extends UiMessageFormatter
    ? Key
    : never;
}[UiMessageKey];

export type UiStaticMessageKey = Exclude<UiMessageKey, UiFormattedMessageKey>;

type UiFormattedMessageArguments<Key extends UiFormattedMessageKey> =
  EnglishMessages[Key] extends (...parameters: infer Arguments) => string
    ? Arguments
    : never;

export function uiText(key: UiStaticMessageKey): string;
export function uiText<Key extends UiFormattedMessageKey>(
  key: Key,
  ...args: UiFormattedMessageArguments<Key>
): string;
export function uiText(key: UiMessageKey, ...args: unknown[]): string {
  const message = englishMessages[key];

  if (typeof message === "function") {
    // The overloads preserve exact key/argument checking for callers. This
    // implementation only dispatches values from the private typed catalog.
    const formatMessage = message as unknown as (
      ...parameters: unknown[]
    ) => string;

    return formatMessage(...args);
  }

  return message;
}

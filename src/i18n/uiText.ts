const englishMessages = {
  "app.name": "VRC Live Caption",
  "app.mode.outgoingCaption": "Outgoing Caption",

  "common.loading": "loading",

  "navigation.primary": "Primary",
  "navigation.captioning": "Captioning",
  "navigation.settings": "Settings",
  "navigation.diagnostics": "Diagnostics",

  "audio.devices.default": "Default device",
  "audio.devices.defaultInput": "Default input device",
  "audio.devices.defaultNamed": ({ name }: { name: string }) =>
    `${name} (default)`,
  "audio.devices.savedDisconnected": "Saved device (not connected)",
  "audio.level.reading": ({
    peakDbfs,
    rmsDbfs,
  }: {
    peakDbfs: number;
    rmsDbfs: number;
  }) => `RMS ${rmsDbfs.toFixed(1)} dBFS · Peak ${peakDbfs.toFixed(1)} dBFS`,

  "serviceProvider.openai": "OpenAI",
  "recognition.path.gptTranscribe": "GPT Transcribe",
  "recognition.path.gptLiveTranscribe": "GPT Live Transcribe",
  "recognition.path.gptTranscribe.description":
    "Transcription begins after each speech unit is committed. Supports Completed publication.",
  "recognition.path.gptLiveTranscribe.description":
    "Low-latency transcription while you speak. Supports both Completed and Live publication.",

  "publication.mode.completed": "Completed",
  "publication.mode.live": "Live",
  "publication.option.completed.description":
    "Send each caption only after its unit completes.",
  "publication.option.live.description":
    "Update the newest caption while speech continues, when the recognition path supports it.",
  "publication.timing.completed": "Sends completed captions only.",
  "publication.timing.liveUnit": ({ delayMs }: { delayMs: number }) =>
    `Observes the first ${String(delayMs)} ms, then updates the newest caption until its unit completes.`,
  "translation.content.sourceOnly": "Source only",
  "translation.content.translationOnly": "Translation only",
  "translation.content.bilingual": "Bilingual",
  "translation.target.en": "English (en)",
  "translation.target.zhHans": "Simplified Chinese (zh-Hans)",
  "translation.endpoint.official": "Official OpenAI",
  "translation.endpoint.custom": "Custom HTTPS endpoint",
  "translation.failure.providerAuthenticationFailed":
    "The Translation service rejected its credential.",
  "translation.failure.providerPermissionDenied":
    "The Translation service denied permission for this request.",
  "translation.failure.providerInvalidRequest":
    "The Translation service rejected this request as invalid.",
  "translation.failure.providerRateLimited":
    "The Translation service rate-limited this request.",
  "translation.failure.providerUsageLimit":
    "The Translation service usage limit was reached.",
  "translation.failure.providerUnavailable":
    "The Translation service was unavailable.",
  "translation.failure.invalidOutput":
    "The Translation service returned unusable output.",
  "translation.failure.deadlineExceeded":
    "Translation did not finish before its deadline.",
  "translation.failure.backpressure":
    "Translation capacity was full for this caption.",
  "translation.failure.sourceTooLarge":
    "This Source caption was too large to translate safely.",
  "translation.failure.stopped":
    "Translation stopped before this caption finished.",
  "translation.failure.failed": "Translation could not complete this caption.",
  "runtime.title": "Runtime",
  "runtime.status.idle": "Idle",
  "runtime.status.starting": "Starting",
  "runtime.status.running": "Running",
  "runtime.status.reconnecting": "Reconnecting",
  "runtime.status.stopping": "Stopping",
  "runtime.status.stopped": "Stopped",
  "runtime.status.error": "Error",
  "runtime.status.initialIdleMessage": "Runtime is idle",
  "runtime.status.noMessage": "No runtime status message.",
  "runtime.actions.start": "Start",
  "runtime.actions.stop": "Stop",
  "runtime.actions.oscTest": "OSC Test",
  "runtime.errors.actionFailed": "Runtime action failed",
  "runtime.errors.unknownAction": "Action failed.",
  "runtime.errors.desktopRequired":
    "This feature requires the Tauri desktop app.",

  "caption.preview.eyebrow": "Caption Preview",
  "caption.preview.title": "Current output",
  "caption.previewStatus.waiting": "Waiting",
  "caption.previewStatus.listening": "Listening",
  "caption.previewStatus.ongoing": "Ongoing",
  "caption.previewStatus.completed": "Completed",
  "caption.state.waiting": "Waiting for caption events.",
  "caption.completedAnnouncement": ({ text }: { text: string }) =>
    `Completed caption: ${text}`,

  "diagnostics.title": "Diagnostics",
  "diagnostics.page.title": "Runtime events and captions",
  "diagnostics.empty": "No diagnostics yet.",
  "diagnostics.completedCaptions.title": "Completed captions",
  "diagnostics.completedCaptions.empty": "No completed caption events yet.",
  "diagnostics.report.copy": "Copy diagnostic report",
  "diagnostics.report.copying": "Copying diagnostic report…",
  "diagnostics.report.copied": "Diagnostic report copied",
  "diagnostics.report.copyFailed": "Could not copy diagnostic report",
  "diagnostics.severity.info": "Info",
  "diagnostics.severity.warning": "Warning",
  "diagnostics.severity.error": "Error",
  "diagnostics.category.config": "Config",
  "diagnostics.category.runtime": "Runtime",
  "diagnostics.category.audio": "Audio",
  "diagnostics.category.stt": "Recognition",
  "diagnostics.category.osc": "OSC",

  "captioning.eyebrow": "Captioning",
  "captioning.title": "Speak, preview, send captions",
  "captioning.chatbox.on": "Chatbox on",
  "captioning.chatbox.off": "Chatbox off",
  "captioning.chatbox.unavailable": "Chatbox unavailable",
  "captioning.currentSetup.title": "Current setup",
  "captioning.currentSetup.activeGenerationTitle": "Current run setup",
  "captioning.currentSetup.failedGenerationTitle": "Failed run setup",
  "captioning.currentSetup.nextStartTitle": "Next Start setup",
  "captioning.currentSetup.nextStartBadge": "Next Start",
  "captioning.currentSetup.pendingChanges.title": "Saved changes are pending",
  "captioning.currentSetup.pendingChanges.description":
    "The current run is unchanged. Saved settings will apply after Stop and the next Start.",
  "captioning.currentSetup.pendingChanges.failedDescription":
    "The failed runtime generation is retained for diagnostics. Saved settings will be used on the next Start.",
  "captioning.currentSetup.edit": "Edit",
  "captioning.currentSetup.microphone": "Microphone",
  "captioning.currentSetup.recognitionPath": "Recognition path",
  "captioning.currentSetup.publication": "Publication",
  "captioning.currentSetup.translation": "Translation",
  "captioning.currentSetup.translationValue": ({
    content,
    endpoint,
    target,
  }: {
    content: string;
    endpoint: string;
    target: string;
  }) => `${content} · ${target} · ${endpoint}`,
  "captioning.currentSetup.oscTarget": "OSC / Test target",
  "captioning.publication.readyValue": ({
    description,
    mode,
  }: {
    description: string;
    mode: string;
  }) => `${mode} · ${description}`,
  "captioning.publication.incompatibleValue": ({ mode }: { mode: string }) =>
    `${mode} · incompatible for next Start`,
  "captioning.publication.blocked.title":
    "Next Start needs a compatible publication plan",
  "captioning.publication.blocked.description":
    "The saved timing remains selected. In Settings, choose a supported timing or a different recognition service/path.",
  "captioning.publication.blocked.action": "Review Settings",
  "captioning.publication.unavailable": "Loading",
  "captioning.recentActivity.title": "Recent activity",
  "captioning.recentActivity.open": "Open",
  "captioning.recentActivity.latestCompletedCaption":
    "Latest completed caption",
  "captioning.recentActivity.noCompletedCaption": "No completed caption yet.",
  "captioning.recentActivity.latestDiagnostic": "Latest diagnostic",
  "captioning.microphoneMeter.title": "Microphone level",
  "captioning.microphoneMeter.accessibleLabel": "Live microphone level",
  "captioning.microphoneMeter.accessibleValue": ({
    reading,
    status,
  }: {
    reading: string;
    status: string;
  }) => `${reading}. ${status}.`,
  "captioning.microphoneMeter.accessibleStatuses": ({
    clippingStatus,
    gateStatus,
  }: {
    clippingStatus: string;
    gateStatus: string;
  }) => `${gateStatus}. ${clippingStatus}`,
  "captioning.microphoneMeter.waiting": "Waiting for microphone audio",
  "captioning.microphoneMeter.reconnecting": "Paused while reconnecting",
  "captioning.microphoneMeter.stopping": "Paused while the runtime stops",
  "captioning.microphoneMeter.gateOpen": "Speech gate open",
  "captioning.microphoneMeter.belowThreshold": "Below speech threshold",
  "captioning.microphoneMeter.clipping": "Clipping detected",
  "captioning.translationActivity.title": "Translation activity",
  "captioning.translationActivity.description":
    "Each completed caption and its Translation for the current run, newest first.",
  "captioning.translationActivity.status.inactive": "Translation inactive",
  "captioning.translationActivity.status.active": "Translation active",
  "captioning.translationActivity.status.degraded": "Translation degraded",
  "captioning.translationActivity.degradedDescription":
    "Recognition and the selected content continue unchanged. Captions that already failed are not retried; later captions are still translated.",
  "captioning.translationActivity.noUnits": "Waiting for a completed caption.",
  "captioning.translationActivity.unitsLabel":
    "Translation for the current run",
  "captioning.translationActivity.sourceLabel": "Source",
  "captioning.translationActivity.translationLabel": "Translation",
  "captioning.translationActivity.unit.pending": "Translating",
  "captioning.translationActivity.unit.pendingDescription":
    "Waiting for the exact Translation to finish.",
  "captioning.translationActivity.unit.completed": "Translated",
  "captioning.translationActivity.unit.failed": "Translation failed",
  "captioning.translationActivity.unit.failedTranslationOnly":
    "Chatbox skips this caption.",
  "captioning.translationActivity.unit.failedBilingual":
    "Chatbox shows only the Source for this caption.",

  "settings.title": "Settings",
  "settings.page.title": "Capture, recognition, and output",
  "settings.description":
    "Configure capture, service credentials, and Chatbox output.",
  "settings.actions.refreshDevices": "Refresh devices",
  "settings.actions.save": "Save Settings",
  "settings.microphoneTest.action": "Test microphone",
  "settings.microphoneTest.runningAction": "Testing microphone…",
  "settings.microphoneTest.pending":
    "Listening to the selected microphone for about two seconds…",
  "settings.microphoneTest.runtimeActive":
    "Stop the runtime before testing this microphone.",
  "settings.microphoneTest.heard": "Audio is above the speech threshold",
  "settings.microphoneTest.belowThreshold":
    "Audio is below the speech threshold",
  "settings.microphoneTest.clipping": "Clipping detected",
  "settings.microphoneTest.errorTitle": "Microphone test failed",
  "settings.microphoneTest.unknownError": "Microphone probe failed.",
  "settings.unsavedChanges.confirmLeave":
    "Discard the unsaved settings changes?",
  "settings.errors.actionFailed": "Settings action failed",
  "settings.feedback.saved": "Settings saved",
  "settings.feedback.nextStart.title": "Saved for the next Start",
  "settings.feedback.nextStart.description": ({
    changes,
  }: {
    changes: string;
  }) =>
    `The current run is unchanged. Saved changes to ${changes} will take effect after Stop and the next Start.`,
  "settings.feedback.nextStart.failedDescription": ({
    changes,
  }: {
    changes: string;
  }) =>
    `The failed runtime generation is retained for diagnostics. Saved changes to ${changes} will be used on the next Start.`,
  "settings.feedback.nextStart.change.microphone": "microphone",
  "settings.feedback.nextStart.change.recognition": "speech recognition",
  "settings.feedback.nextStart.change.translation": "translation",
  "settings.feedback.nextStart.change.credential": "service credential",
  "settings.feedback.nextStart.change.chatboxOutput": "Chatbox output",
  "settings.feedback.nextStart.change.publication":
    "caption content or publication timing",
  "settings.sections.audio": "Audio",
  "settings.sections.recognition": "Speech recognition",
  "settings.sections.chatboxOutput": "Chatbox output",
  "settings.sections.serviceCredentials": "Service credentials",
  "settings.fields.microphone": "Microphone",
  "settings.fields.language": "Expected languages",
  "settings.fields.language.description":
    "Add one or more language hints, such as zh, en, or ja.",
  "settings.fields.language.required":
    "Add at least one non-empty language hint without duplicates.",
  "settings.fields.recognitionPath": "Recognition path",
  "settings.fields.oscHost": "OSC host",
  "settings.fields.port": "Port",
  "settings.fields.chatboxOutput": "Chatbox output",
  "settings.fields.ongoingPreview": "App ongoing preview",
  "settings.fields.publicationMode": "Publication timing",
  "settings.publication.description":
    "Choose when captions are sent. The app validates compatibility when you Save.",
  "settings.publication.loading": "Waiting for the caption pipeline plan.",
  "settings.publication.ready": ({ description }: { description: string }) =>
    `Plan: ${description}`,
  "settings.publication.unverified.title": "Save to validate this timing",
  "settings.publication.unverified.description":
    "This form has unsaved changes. After Save, the app will validate the recognition path and publication timing together.",
  "settings.publication.incompatible.title": ({ mode }: { mode: string }) =>
    `${mode} is not supported by this recognition path`,
  "settings.publication.incompatible.description":
    "The saved choice has not been changed. Choose which experience to keep, then Save.",
  "settings.publication.incompatible.useMode": ({ mode }: { mode: string }) =>
    `Keep current path · Use ${mode}`,
  "settings.publication.incompatible.changePath": ({
    mode,
  }: {
    mode: string;
  }) => `Keep ${mode} · Change recognition path`,
  "settings.translation.title": "Completed Translation",
  "settings.translation.description":
    "Choose which completed caption lanes to publish and how Translation is prepared.",
  "settings.translation.content.legend": "Caption content",
  "settings.translation.content.description":
    "Translation choices are available for Completed publication only.",
  "settings.translation.content.sourceOnly": "Source only",
  "settings.translation.content.sourceOnly.description":
    "Publish completed Source text.",
  "settings.translation.content.translationOnly": "Translation only",
  "settings.translation.content.translationOnly.description":
    "Publish completed Translation text without Source text.",
  "settings.translation.content.bilingual": "Bilingual",
  "settings.translation.content.bilingual.description":
    "Publish Source above Translation in one Chatbox view.",
  "settings.translation.inactive.title": "Translation inactive",
  "settings.translation.inactive.description":
    "Saved Translation choices remain dormant while Source-only content is selected.",
  "settings.translation.path":
    "Uses the fixed Responses completed-text Translation path.",
  "settings.translation.target.legend": "Translation target",
  "settings.translation.target.description":
    "Choose explicitly; the app never infers this from UI language, recognition hints, or Source text.",
  "settings.translation.target.required": "Choose a Translation target.",
  "settings.translation.target.en": "English (en)",
  "settings.translation.target.zhHans": "Simplified Chinese (zh-Hans)",
  "settings.translation.endpoint.legend": "Translation endpoint",
  "settings.translation.endpoint.official": "Official OpenAI",
  "settings.translation.endpoint.official.description":
    "Reuse the existing OpenAI credential without copying it into App Config.",
  "settings.translation.endpoint.custom": "Custom HTTPS endpoint",
  "settings.translation.endpoint.custom.description":
    "Use a separate Custom Translation credential for this endpoint.",
  "settings.translation.officialDisclosure.title":
    "Official Translation data use",
  "settings.translation.officialDisclosure.description":
    "Completed Source text is sent to OpenAI for Translation. Requests use store: false; that setting does not by itself provide Zero Data Retention. Official Translation uses the existing OpenAI credential.",
  "settings.translation.customDisclosure.title": "Custom Translation data use",
  "settings.translation.customDisclosure.description":
    "Completed Source text and the separate Custom Translation credential are sent to the selected endpoint. store: false does not define or guarantee the operator's retention policy. Recognition audio is not rerouted to this endpoint. The Official OpenAI credential is not sent.",
  "settings.translation.customApiBaseUrl": "Custom API base URL",
  "settings.translation.customApiBaseUrl.description":
    "Enter the HTTPS base URL only. VRC Live Caption appends one responses path segment.",
  "settings.translation.customApiBaseUrl.placeholder":
    "https://gateway.example/v1",
  "settings.translation.customApiBaseUrl.error.invalidUrl":
    "API base URL must be a valid URL without whitespace or control characters.",
  "settings.translation.customApiBaseUrl.error.httpsRequired":
    "API base URL must use HTTPS.",
  "settings.translation.customApiBaseUrl.error.hostRequired":
    "API base URL must include a host.",
  "settings.translation.customApiBaseUrl.error.userInformationForbidden":
    "API base URL cannot contain user information.",
  "settings.translation.customApiBaseUrl.error.queryOrFragmentForbidden":
    "API base URL cannot contain a query or fragment.",
  "settings.translation.customApiBaseUrl.error.invalidPercentEncoding":
    "API base URL must contain valid percent encoding.",
  "settings.translation.customApiBaseUrl.error.responsesEndpointForbidden":
    "Enter the API base URL without the Responses endpoint.",
  "settings.translation.credentialStatus.openai": "OpenAI credential",
  "settings.translation.credentialStatus.custom":
    "Custom Translation credential",
  "settings.translation.nextStart":
    "Saved changes take effect on the next Start and do not change the current run.",
  "settings.translation.liveIncompatible.title":
    "Translation content requires Completed publication",
  "settings.translation.liveIncompatible.description":
    "Keep both choices explicit, or switch publication timing to Completed before Start.",
  "settings.translation.liveIncompatible.action": "Use Completed",
  "settings.loading": "Loading settings...",
  "settings.loadFailed": "Settings could not be loaded.",
  "settings.credentials.description":
    "Credentials are stored separately from App Config and are never shown again in plaintext.",
  "settings.credentials.status.checking": "Checking",
  "settings.credentials.status.notSaved": "Not saved",
  "settings.credentials.status.unavailable": "Unavailable",
  "settings.credentials.status.environment": ({
    displaySuffix,
  }: {
    displaySuffix: string | null;
  }) => `Environment ${displaySuffix ? `...${displaySuffix}` : "configured"}`,
  "settings.credentials.status.system": ({
    displaySuffix,
  }: {
    displaySuffix: string | null;
  }) => `System ${displaySuffix ? `...${displaySuffix}` : "configured"}`,
  "settings.credentials.openai.title": "OpenAI credentials",
  "settings.credentials.openai.cloudDisclosure":
    "When OpenAI cloud speech recognition is selected, microphone audio is uploaded to OpenAI for transcription.",
  "settings.credentials.openai.apiKey": "API key",
  "settings.credentials.openai.apiKeyPlaceholder": "sk-...",
  "settings.credentials.openai.actions.save": "Save Key",
  "settings.credentials.openai.actions.replace": "Replace key",
  "settings.credentials.openai.actions.remove": "Remove key",
  "settings.credentials.openai.errors.actionFailed": "API key action failed",
  "settings.credentials.openai.removeDialog.title": "Remove OpenAI API key?",
  "settings.credentials.openai.removeDialog.description":
    "The saved key will be removed from the system credential store. You can add it again later.",
  "settings.credentials.openai.removeDialog.currentGenerationDescription":
    "The saved key will be removed from the system credential store. The current run keeps the credential captured at Start until you Stop the runtime.",
  "settings.credentials.openai.removeDialog.cancel": "Cancel",
  "settings.credentials.openai.removeDialog.confirm": "Remove API key",
  "settings.credentials.customTranslation.title":
    "Custom Translation credentials",
  "settings.credentials.customTranslation.disclosure":
    "This separate key is sent only to the selected Custom Translation endpoint.",
  "settings.credentials.customTranslation.apiKey": "API key",
  "settings.credentials.customTranslation.apiKeyPlaceholder":
    "Custom Translation API key",
  "settings.credentials.customTranslation.actions.save": "Save key",
  "settings.credentials.customTranslation.actions.replace": "Replace key",
  "settings.credentials.customTranslation.actions.remove": "Remove key",
  "settings.credentials.customTranslation.errors.actionFailed":
    "Custom credential action failed",
  "settings.credentials.customTranslation.removeDialog.title":
    "Remove Custom Translation API key?",
  "settings.credentials.customTranslation.removeDialog.description":
    "The saved key will be removed from the system credential store. You can add it again later.",
  "settings.credentials.customTranslation.removeDialog.currentGenerationDescription":
    "The saved key will be removed from the system credential store. The current run keeps the credential captured at Start until you Stop the runtime.",
  "settings.credentials.customTranslation.removeDialog.cancel": "Cancel",
  "settings.credentials.customTranslation.removeDialog.confirm":
    "Remove API key",

  "notFound.eyebrow": "Page not found",
  "notFound.title": "This route does not exist",
  "notFound.backToCaptioning": "Back to Captioning",
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

import type {
  ContentSelection,
  PublicationMode,
  RecognitionPath,
  TranslationPath,
  TranslationTarget,
} from "./captionPipeline";

export const APP_CONFIG_SCHEMA_VERSION = 2 as const;

export type TranslationEndpoint =
  | Readonly<{ kind: "official" }>
  | Readonly<{ kind: "custom"; apiBaseUrl: string }>;

export type TranslationConfig = Readonly<{
  path: TranslationPath;
  target: TranslationTarget;
  endpoint: TranslationEndpoint;
}>;

export type AppConfig = {
  schemaVersion: typeof APP_CONFIG_SCHEMA_VERSION;
  audio: {
    inputDeviceId: string | null;
  };
  recognition: {
    path: RecognitionPath;
    expectedLanguages: string[];
  };
  translation: TranslationConfig | null;
  osc: {
    host: string;
    port: number;
    enabled: boolean;
  };
  publication: {
    mode: PublicationMode;
    content: ContentSelection;
  };
  ui: {
    showOngoingPreview: boolean;
  };
};

export type TranslationApiBaseUrlValidationReason =
  | "invalidUrl"
  | "httpsRequired"
  | "hostRequired"
  | "userinfoForbidden"
  | "queryOrFragmentForbidden"
  | "invalidPercentEncoding"
  | "responsesPathForbidden";

export function translationApiBaseUrlValidationReason(
  raw: string,
): TranslationApiBaseUrlValidationReason | null {
  // URL parsers normalize an empty userinfo marker away, so inspect the raw
  // authority before parsing to reject every syntactic userinfo form.
  const schemeSeparator = raw.indexOf("://");
  const authority =
    schemeSeparator < 0
      ? ""
      : (raw
          .slice(schemeSeparator + 3)
          .split(/[/?#]/u, 1)
          .at(0) ?? "");
  if (authority.includes("@")) {
    return "userinfoForbidden";
  }

  let parsed: URL;
  try {
    parsed = new URL(raw);
  } catch {
    return "invalidUrl";
  }
  if (parsed.protocol !== "https:") {
    return "httpsRequired";
  }
  if (parsed.hostname.length === 0) {
    return "hostRequired";
  }
  if (parsed.username.length > 0 || parsed.password.length > 0) {
    return "userinfoForbidden";
  }
  if (raw.includes("?") || raw.includes("#")) {
    return "queryOrFragmentForbidden";
  }

  const finalSegment = parsed.pathname
    .split("/")
    .filter((segment) => segment.length > 0)
    .at(-1);
  if (finalSegment !== undefined) {
    let decodedFinalSegment: string;
    try {
      decodedFinalSegment = decodeURIComponent(finalSegment);
    } catch {
      return "invalidPercentEncoding";
    }
    if (decodedFinalSegment.toLocaleLowerCase("en") === "responses") {
      return "responsesPathForbidden";
    }
  }

  return null;
}

export function translationApiBaseUrlValidationError(
  raw: string,
): string | null {
  switch (translationApiBaseUrlValidationReason(raw)) {
    case null:
      return null;
    case "invalidUrl":
      return "API base URL must be a valid URL.";
    case "httpsRequired":
      return "API base URL must use HTTPS.";
    case "hostRequired":
      return "API base URL must include a host.";
    case "userinfoForbidden":
      return "API base URL cannot contain user information.";
    case "queryOrFragmentForbidden":
      return "API base URL cannot contain a query or fragment.";
    case "invalidPercentEncoding":
      return "API base URL must contain valid percent encoding.";
    case "responsesPathForbidden":
      return "API base URL cannot include the Responses endpoint.";
  }
}

export function appConfigValidationError(config: AppConfig): string | null {
  if (config.recognition.expectedLanguages.length === 0) {
    return "At least one expected recognition language is required.";
  }

  const normalizedLanguages = new Set<string>();
  for (const language of config.recognition.expectedLanguages) {
    const normalized = language.trim().toLocaleLowerCase("en");
    if (normalized.length === 0) {
      return "Expected recognition languages cannot contain an empty value.";
    }
    if (normalizedLanguages.has(normalized)) {
      return "Expected recognition languages cannot contain duplicates.";
    }
    normalizedLanguages.add(normalized);
  }

  if (config.osc.host.trim().length === 0) {
    return "OSC host cannot be empty.";
  }
  if (
    config.publication.content !== "sourceOnly" &&
    config.translation === null
  ) {
    return "Translation content requires a translation selection.";
  }
  if (config.translation?.endpoint.kind === "custom") {
    return translationApiBaseUrlValidationError(
      config.translation.endpoint.apiBaseUrl,
    );
  }

  return null;
}

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

const translationApiBaseUrlValidationMessages = {
  invalidUrl: "API base URL must be a valid URL.",
  httpsRequired: "API base URL must use HTTPS.",
  hostRequired: "API base URL must include a host.",
  userInformationForbidden: "API base URL cannot contain user information.",
  queryOrFragmentForbidden: "API base URL cannot contain a query or fragment.",
  invalidPercentEncoding: "API base URL must contain valid percent encoding.",
  responsesEndpointForbidden:
    "API base URL cannot include the Responses endpoint.",
} as const;

export type TranslationApiBaseUrlValidationReason =
  keyof typeof translationApiBaseUrlValidationMessages;

type TranslationApiBaseUrlValidationPolicy = "appConfigV2" | "newEdit";

function translationApiBaseUrlValidationReason(
  raw: string,
  policy: TranslationApiBaseUrlValidationPolicy,
): TranslationApiBaseUrlValidationReason | null {
  if (policy === "newEdit") {
    let containsControlCharacter = false;
    for (let index = 0; index < raw.length; index += 1) {
      const codeUnit = raw.charCodeAt(index);
      if (codeUnit <= 0x1f || (codeUnit >= 0x7f && codeUnit <= 0x9f)) {
        containsControlCharacter = true;
        break;
      }
    }
    if (raw.trim() !== raw || containsControlCharacter) {
      return "invalidUrl";
    }
  }

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
    return "userInformationForbidden";
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
    return "userInformationForbidden";
  }
  if (raw.includes("?") || raw.includes("#")) {
    return "queryOrFragmentForbidden";
  }

  const pathSegments = parsed.pathname.split("/");
  const finalSegment = pathSegments
    .filter((segment) => segment.length > 0)
    .at(-1);
  const segmentsToValidate =
    policy === "newEdit"
      ? pathSegments
      : finalSegment === undefined
        ? []
        : [finalSegment];
  let decodedFinalSegment = "";
  for (const segment of segmentsToValidate) {
    let decodedSegment: string;
    try {
      decodedSegment = decodeURIComponent(segment);
    } catch {
      return "invalidPercentEncoding";
    }

    if (decodedSegment.length > 0) {
      decodedFinalSegment = decodedSegment;
    }
  }
  if (decodedFinalSegment.toLocaleLowerCase("en") === "responses") {
    return "responsesEndpointForbidden";
  }

  return null;
}

function translationApiBaseUrlValidationError(
  raw: string,
  policy: TranslationApiBaseUrlValidationPolicy,
): string | null {
  const reason = translationApiBaseUrlValidationReason(raw, policy);
  return reason === null
    ? null
    : translationApiBaseUrlValidationMessages[reason];
}

// App Config V2 already accepted URL-parser normalization and only verified
// the final path segment. Persisted and runtime payloads keep that contract.
export function translationApiBaseUrlV2ValidationError(
  raw: string,
): string | null {
  return translationApiBaseUrlValidationError(raw, "appConfigV2");
}

// New settings edits use stricter syntax without retroactively invalidating
// App Config V2 values that the application has already persisted.
export function translationApiBaseUrlNewEditValidationReason(
  raw: string,
): TranslationApiBaseUrlValidationReason | null {
  return translationApiBaseUrlValidationReason(raw, "newEdit");
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
    return translationApiBaseUrlV2ValidationError(
      config.translation.endpoint.apiBaseUrl,
    );
  }

  return null;
}

export const UI_LOCALES = ["en", "zh-Hans"] as const;

export type UiLocale = (typeof UI_LOCALES)[number];

export function uiLocaleFromLanguage(language: string): UiLocale {
  const normalized = language.trim().toLocaleLowerCase("en");

  return normalized === "zh" ||
    normalized.startsWith("zh-cn") ||
    normalized.startsWith("zh-sg") ||
    normalized.startsWith("zh-hans")
    ? "zh-Hans"
    : "en";
}

export function currentUiLocale(): UiLocale {
  const language = typeof navigator === "undefined" ? "en" : navigator.language;
  const previewSearch =
    import.meta.env.DEV && typeof location !== "undefined"
      ? location.search
      : "";

  return uiLocaleFromPreview(previewSearch, language);
}

export function uiLocaleFromPreview(
  search: string,
  language: string,
): UiLocale {
  const requested = new URLSearchParams(search).get("uiLocale");

  return requested === "en" || requested === "zh-Hans"
    ? requested
    : uiLocaleFromLanguage(language);
}

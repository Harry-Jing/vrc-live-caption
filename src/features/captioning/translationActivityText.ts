import type { UiLocale } from "../../i18n/uiLocale";
import type { TranslationFailureReason } from "../../runtime/captionAggregate";

const english = {
  title: "Translation activity",
  description:
    "Authoritative progress for the current runtime generation, paired to each exact Source completion.",
  inactive: "Translation stopped",
  active: "Translation active",
  degraded: "Translation degraded",
  contentLabel: "Selected content",
  targetLabel: "Translation target",
  endpointLabel: "Endpoint",
  degradedDescription:
    "Recognition continues. Failed units stay terminal, and the selected content is not changed.",
  noUnits: "Waiting for a completed Source caption.",
  contentSourceOnly: "Source only",
  contentTranslationOnly: "Translation only",
  contentBilingual: "Bilingual",
  targetEnglish: "English",
  targetSimplifiedChinese: "Simplified Chinese",
  endpointOfficial: "Official",
  endpointCustom: "Custom",
  unitsLabel: "Current generation Translation units",
  sourceLabel: "Source",
  translationLabel: "Translation",
  pending: "Translating",
  pendingDescription: "Waiting for the exact Translation to finish.",
  completed: "Translated",
  failed: "Translation failed",
  failedTranslationOnly:
    "This caption remains omitted because the selected Translation failed.",
  providerAuthenticationFailed:
    "The Translation service rejected its credential.",
  providerPermissionDenied:
    "The Translation service denied permission for this request.",
  providerInvalidRequest:
    "The Translation service rejected this request as invalid.",
  providerRateLimited: "The Translation service rate-limited this request.",
  providerUsageLimit: "The Translation service usage limit was reached.",
  providerUnavailable: "The Translation service was unavailable.",
  invalidOutput: "The Translation service returned unusable output.",
  deadlineExceeded: "Translation did not finish before its deadline.",
  backpressure: "Translation capacity was full for this caption.",
  sourceTooLarge: "This Source caption was too large to translate safely.",
  stopped: "Translation stopped before this caption finished.",
  failedGeneric: "Translation could not complete this caption.",
} as const;

export type TranslationActivityTextKey = keyof typeof english;

const simplifiedChinese: Record<TranslationActivityTextKey, string> = {
  title: "翻译活动",
  description: "当前运行代的权威进度，并与每条确切的已完成原文对应。",
  inactive: "翻译已停止",
  active: "翻译运行中",
  degraded: "翻译已降级",
  contentLabel: "所选内容",
  targetLabel: "翻译目标",
  endpointLabel: "端点",
  degradedDescription:
    "语音识别会继续。失败单元保持终态，所选内容不会被自动更改。",
  noUnits: "正在等待已完成的原文字幕。",
  contentSourceOnly: "仅原文",
  contentTranslationOnly: "仅译文",
  contentBilingual: "双语",
  targetEnglish: "英语",
  targetSimplifiedChinese: "简体中文",
  endpointOfficial: "官方",
  endpointCustom: "自定义",
  unitsLabel: "当前运行代的翻译单元",
  sourceLabel: "原文",
  translationLabel: "译文",
  pending: "翻译中",
  pendingDescription: "正在等待对应译文完成。",
  completed: "翻译完成",
  failed: "翻译失败",
  failedTranslationOnly: "由于所选译文失败，此条字幕会继续省略。",
  providerAuthenticationFailed: "翻译服务拒绝了所用凭据。",
  providerPermissionDenied: "翻译服务拒绝执行此请求。",
  providerInvalidRequest: "翻译服务判定此请求无效。",
  providerRateLimited: "翻译服务限制了此请求的速率。",
  providerUsageLimit: "已达到翻译服务的用量上限。",
  providerUnavailable: "翻译服务暂时不可用。",
  invalidOutput: "翻译服务返回了无法使用的结果。",
  deadlineExceeded: "翻译未能在截止时间前完成。",
  backpressure: "此条字幕提交时翻译容量已满。",
  sourceTooLarge: "此条原文过大，无法安全翻译。",
  stopped: "此条字幕完成翻译前，翻译已停止。",
  failedGeneric: "此条字幕未能完成翻译。",
};

const failureTextKeys: Record<
  TranslationFailureReason,
  TranslationActivityTextKey
> = {
  "translation.provider_authentication_failed": "providerAuthenticationFailed",
  "translation.provider_permission_denied": "providerPermissionDenied",
  "translation.provider_invalid_request": "providerInvalidRequest",
  "translation.provider_rate_limited": "providerRateLimited",
  "translation.provider_usage_limit": "providerUsageLimit",
  "translation.provider_unavailable": "providerUnavailable",
  "translation.invalid_output": "invalidOutput",
  "translation.deadline_exceeded": "deadlineExceeded",
  "translation.backpressure": "backpressure",
  "translation.source_too_large": "sourceTooLarge",
  "translation.stopped": "stopped",
  "translation.failed": "failedGeneric",
};

export function translationActivityText(
  locale: UiLocale,
  key: TranslationActivityTextKey,
): string {
  return locale === "zh-Hans" ? simplifiedChinese[key] : english[key];
}

export function translationFailureText(
  locale: UiLocale,
  reason: TranslationFailureReason,
): string {
  return translationActivityText(locale, failureTextKeys[reason]);
}

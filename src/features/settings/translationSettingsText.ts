import type { UiLocale } from "../../i18n/uiLocale";

const english = {
  title: "Translation",
  description: "Choose which Completed caption content is sent to Chatbox.",
  contentLabel: "Completed content",
  sourceOnly: "Source only",
  sourceOnlyDescription: "Send the recognized Source text.",
  translationOnly: "Translation only",
  translationOnlyDescription:
    "Wait for Translation and send it only when it succeeds.",
  bilingual: "Bilingual",
  bilingualDescription: "Send Source above its exact Translation.",
  dormantTitle: "Translation is dormant",
  dormantDescription:
    "Saved Translation choices remain available, but Source-only does not use a Translation credential or upload Source text.",
  nextStartTitle: "Applies on the next Start",
  nextStartDescription:
    "Saving does not change the current run. Stop and Start again to use these choices.",
  targetLabel: "Translation target",
  targetDescription:
    "Choose explicitly. The app never infers this from the UI language, recognition hints, or Source text.",
  targetRequired: "Choose English or Simplified Chinese.",
  targetEnglish: "English",
  targetSimplifiedChinese: "Simplified Chinese",
  endpointLabel: "Translation endpoint",
  endpointOfficial: "Official",
  endpointOfficialDescription: "Use OpenAI Responses.",
  endpointCustom: "Custom",
  endpointCustomDescription: "Use a compatible HTTPS API base URL.",
  customUrlLabel: "Custom API base URL",
  customUrlPlaceholder: "https://translation.example.com/v1",
  customUrlDescription:
    "Enter the base URL only; the app appends exactly one /responses segment.",
  customUrlInvalid: "Enter a valid URL.",
  customUrlHttpsRequired: "The API base URL must use HTTPS.",
  customUrlHostRequired: "The API base URL must include a host.",
  customUrlUserinfoForbidden:
    "The API base URL cannot contain user information.",
  customUrlQueryOrFragmentForbidden:
    "The API base URL cannot contain a query or fragment.",
  customUrlInvalidPercentEncoding:
    "The API base URL must contain valid percent encoding.",
  customUrlResponsesPathForbidden:
    "Enter the base URL without the Responses endpoint.",
  officialUploadDisclosure:
    "Completed Source text is sent to OpenAI Responses with store: false. The API key is not copied into settings.",
  customUploadDisclosure:
    "Completed Source text is sent to the Custom operator. The app requests store: false, but that does not define the operator's retention policy.",
  officialCredentialTitle: "OpenAI credential",
  officialCredentialDescription:
    "Official Translation reuses the OpenAI credential managed in Speech recognition above.",
  customCredentialTitle: "Custom Translation credential",
  customCredentialDescription:
    "Stored separately in the system credential store and used only for the Custom endpoint.",
  apiKeyLabel: "API key",
  apiKeyPlaceholder: "Custom endpoint key",
  saveKey: "Save key",
  replaceKey: "Replace key",
  removeKey: "Remove key",
  removeDialogTitle: "Remove Custom Translation API key?",
  removeDialogDescription:
    "The saved key will be removed from the system credential store. You can add it again later.",
  removeDialogCurrentGenerationDescription:
    "The saved key will be removed from the system credential store. The current run keeps the credential captured at Start until you Stop the runtime.",
  cancel: "Cancel",
  checking: "Checking",
  notSaved: "Not saved",
  unavailable: "Unavailable",
  savedInSystem: "Saved in system credential store",
  savedInEnvironment: "Provided by environment",
  credentialActionFailed: "Credential action failed",
} as const;

type TranslationSettingsTextKey = keyof typeof english;

const simplifiedChinese: Record<TranslationSettingsTextKey, string> = {
  title: "翻译",
  description: "选择要发送到 Chatbox 的已完成字幕内容。",
  contentLabel: "已完成内容",
  sourceOnly: "仅原文",
  sourceOnlyDescription: "发送识别出的原文。",
  translationOnly: "仅译文",
  translationOnlyDescription: "等待翻译，仅在翻译成功时发送译文。",
  bilingual: "双语",
  bilingualDescription: "在对应译文上方发送原文。",
  dormantTitle: "翻译处于休眠状态",
  dormantDescription:
    "已保存的翻译选项会保留，但仅原文模式不会使用翻译凭据或上传原文。",
  nextStartTitle: "下次启动时生效",
  nextStartDescription:
    "保存不会更改当前运行。请先停止，再重新启动以使用这些选项。",
  targetLabel: "翻译目标语言",
  targetDescription:
    "请明确选择。应用不会根据界面语言、识别提示或原文推断目标语言。",
  targetRequired: "请选择英语或简体中文。",
  targetEnglish: "英语",
  targetSimplifiedChinese: "简体中文",
  endpointLabel: "翻译端点",
  endpointOfficial: "官方",
  endpointOfficialDescription: "使用 OpenAI Responses。",
  endpointCustom: "自定义",
  endpointCustomDescription: "使用兼容的 HTTPS API 基础 URL。",
  customUrlLabel: "自定义 API 基础 URL",
  customUrlPlaceholder: "https://translation.example.com/v1",
  customUrlDescription: "只输入基础 URL；应用会准确追加一个 /responses 路径。",
  customUrlInvalid: "请输入有效的 URL。",
  customUrlHttpsRequired: "API 基础 URL 必须使用 HTTPS。",
  customUrlHostRequired: "API 基础 URL 必须包含主机名。",
  customUrlUserinfoForbidden: "API 基础 URL 不能包含用户信息。",
  customUrlQueryOrFragmentForbidden: "API 基础 URL 不能包含查询或片段。",
  customUrlInvalidPercentEncoding: "API 基础 URL 必须包含有效的百分号编码。",
  customUrlResponsesPathForbidden: "请输入不含 Responses 端点的基础 URL。",
  officialUploadDisclosure:
    "已完成的原文会发送到 OpenAI Responses，并设置 store: false。API 密钥不会复制到设置中。",
  customUploadDisclosure:
    "已完成的原文会发送给自定义服务方。应用会请求 store: false，但这并不规定服务方的数据保留策略。",
  officialCredentialTitle: "OpenAI 凭据",
  officialCredentialDescription:
    "官方翻译会复用上方语音识别中管理的 OpenAI 凭据。",
  customCredentialTitle: "自定义翻译凭据",
  customCredentialDescription:
    "凭据单独保存在系统凭据存储中，并且仅用于自定义端点。",
  apiKeyLabel: "API 密钥",
  apiKeyPlaceholder: "自定义端点密钥",
  saveKey: "保存密钥",
  replaceKey: "替换密钥",
  removeKey: "移除密钥",
  removeDialogTitle: "移除自定义翻译 API 密钥？",
  removeDialogDescription:
    "已保存的密钥会从系统凭据存储中移除。之后可以重新添加。",
  removeDialogCurrentGenerationDescription:
    "已保存的密钥会从系统凭据存储中移除。当前运行会继续使用启动时捕获的凭据，直到停止运行。",
  cancel: "取消",
  checking: "正在检查",
  notSaved: "未保存",
  unavailable: "不可用",
  savedInSystem: "已保存在系统凭据存储中",
  savedInEnvironment: "由环境变量提供",
  credentialActionFailed: "凭据操作失败",
};

export function translationSettingsText(
  locale: UiLocale,
  key: TranslationSettingsTextKey,
): string {
  return locale === "zh-Hans" ? simplifiedChinese[key] : english[key];
}

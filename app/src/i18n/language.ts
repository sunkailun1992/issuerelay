export type Lang = "zh-CN" | "zh-TW" | "en" | "ja" | "ko";
export const LANGUAGES: Lang[] = ["zh-CN", "zh-TW", "en", "ja", "ko"];

/**
 * The interface language for a `language` setting ("system" or one of LANGUAGES) and the
 * system's locale (the one Rust reports, `AppInfo.system_locale`). For Chinese the script
 * subtag decides first (Hans → zh-CN, Hant → zh-TW), then the region (TW, HK, MO → zh-TW);
 * a language IssueRelay does not offer falls back to English. Keep in step with
 * `resolve_language` in app/src-tauri/src/i18n.rs: both are tested with language-cases.json.
 */
export function resolveLanguage(setting: string, systemLocale: string | undefined): Lang {
  if ((LANGUAGES as string[]).includes(setting)) return setting as Lang;
  if (setting !== "system") return "en";
  const [primary, ...rest] = (systemLocale ?? "").toLowerCase().split(/[-_]/);
  if (primary === "zh") {
    if (rest.includes("hans")) return "zh-CN";
    if (rest.includes("hant")) return "zh-TW";
    return rest.some((tag) => tag === "tw" || tag === "hk" || tag === "mo") ? "zh-TW" : "zh-CN";
  }
  if (primary === "ja") return "ja";
  if (primary === "ko") return "ko";
  return "en";
}

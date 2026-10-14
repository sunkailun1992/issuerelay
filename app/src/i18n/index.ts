import i18n, { type InitOptions } from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import ko from "./locales/ko.json";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import type { Lang } from "./language";

export * from "./language";

const resources = {
  "zh-CN": { translation: zhCN },
  "zh-TW": { translation: zhTW },
  en: { translation: en },
  ja: { translation: ja },
  ko: { translation: ko },
};

/** Starts i18next in `lang`. `options` is for the tests' missing-key guard only. */
export async function initI18n(lang: Lang, options: InitOptions = {}): Promise<void> {
  await i18n.use(initReactI18next).init({
    resources,
    lng: lang,
    fallbackLng: "en",
    interpolation: { escapeValue: false }, // React escapes text nodes itself.
    returnNull: false,
    ...options,
  });
  document.documentElement.lang = lang;
}

/** `initI18n` the first time, `setLanguage` afterwards (tests render many times per file). */
export async function initI18nOnce(lang: Lang): Promise<void> {
  if (i18n.isInitialized) {
    await setLanguage(lang);
  } else {
    await initI18n(lang);
  }
}

export async function setLanguage(lang: Lang): Promise<void> {
  await i18n.changeLanguage(lang);
  document.documentElement.lang = lang;
}

export default i18n;

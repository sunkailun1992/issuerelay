import { afterEach, describe, expect, test } from "vitest";
import i18n, { initI18nOnce, LANGUAGES, setLanguage } from "./index";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import ko from "./locales/ko.json";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";

// test-setup.ts starts i18next in English with the missing-key guard before any test runs, so
// these tests only use what that setup left behind.

describe("the missing-key guard", () => {
  test("a key English lacks fails the test instead of showing the key", () => {
    expect(() => i18n.t("no.such.key")).toThrow("missing translation key: no.such.key");
  });

  test("a key English has is translated and does not trip the guard", () => {
    expect(i18n.t("tray.quit")).toBe(en.tray.quit);
  });

  test("a key that names a group of texts fails the test too, instead of showing a sentence about it", () => {
    expect(() => i18n.t("nav")).toThrow("translation key names a group of texts, not a text: nav");
  });
});

describe("changing the language", () => {
  afterEach(async () => {
    await setLanguage("en");
  });

  test("initI18nOnce switches the language of the started i18next and of the document", async () => {
    await initI18nOnce("ja");
    expect(i18n.language).toBe("ja");
    expect(i18n.t("tray.quit")).toBe(ja.tray.quit);
    expect(document.documentElement.lang).toBe("ja");
  });

  test("the guard is still on after initI18nOnce switched the language", async () => {
    await initI18nOnce("ko");
    expect(i18n.t("tray.quit")).toBe(ko.tray.quit);
    expect(() => i18n.t("no.such.key")).toThrow("missing translation key: no.such.key");
  });

  // Each language must be registered with i18next as well as listed (index.ts `resources`,
  // language.ts `LANGUAGES`): one listed but not registered would fall back to English unnoticed.
  const FILES = { "zh-CN": zhCN, "zh-TW": zhTW, en, ja, ko } as const;
  test.each(LANGUAGES)("setLanguage(%s) gives the panel that language's own file", async (lang) => {
    await setLanguage(lang);
    expect(i18n.language).toBe(lang);
    expect(i18n.getResourceBundle(lang, "translation")).toEqual(FILES[lang]);
    expect(i18n.t("tray.quit")).toBe(FILES[lang].tray.quit);
  });

  test("setLanguage changes the document language too", async () => {
    await setLanguage("zh-TW");
    expect(document.documentElement.lang).toBe("zh-TW");
    expect(i18n.language).toBe("zh-TW");
  });
});

describe("placeholders", () => {
  test("values are filled in as they are: React escapes text nodes itself", () => {
    expect(i18n.t("tray.status_counts", { running: 2, queued: 1 })).toBe("Running 2 · Queued 1");
    expect(i18n.t("tray.status_counts", { running: "<2>", queued: "&" })).toBe("Running <2> · Queued &");
  });
});

import { afterEach, describe, expect, test } from "vitest";
import i18n, { initI18nOnce, setLanguage } from "./index";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import ko from "./locales/ko.json";

// test-setup.ts starts i18next in English with the missing-key guard before any test runs, so
// these tests only use what that setup left behind.

describe("the missing-key guard", () => {
  test("a key English lacks fails the test instead of showing the key", () => {
    expect(() => i18n.t("no.such.key")).toThrow("missing translation key: no.such.key");
  });

  test("a key English has is translated and does not trip the guard", () => {
    expect(i18n.t("tray.quit")).toBe(en.tray.quit);
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

import { describe, expect, test } from "vitest";
import cases from "./language-cases.json";
import { LANGUAGES, resolveLanguage } from "./language";
import zhCN from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import en from "./locales/en.json";
import ja from "./locales/ja.json";
import ko from "./locales/ko.json";

describe("language resolution", () => {
  test.each(cases)("setting $setting with system $system gives $expected", ({ setting, system, expected }) => {
    expect(resolveLanguage(setting, system ?? undefined)).toBe(expected);
  });
});

/** Every key of a nested translation object, dotted. */
function keys(table: object, prefix = ""): string[] {
  return Object.entries(table).flatMap(([key, value]) =>
    typeof value === "object" && value !== null ? keys(value, `${prefix}${key}.`) : [`${prefix}${key}`],
  );
}

function lookup(table: object, key: string): unknown {
  return key.split(".").reduce<unknown>((node, part) => (node as Record<string, unknown>)?.[part], table);
}

const placeholders = (text: string) => [...text.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort();

describe("translation files", () => {
  const files = { "zh-CN": zhCN, "zh-TW": zhTW, en, ja, ko } as const;
  // English is the reference; comparing it with itself proves nothing.
  const others = Object.entries(files).filter(([lang]) => lang !== "en");

  test("there is one file per language", () => {
    expect(Object.keys(files).sort()).toEqual([...LANGUAGES].sort());
  });

  test("English has no empty text", () => {
    for (const key of keys(en)) {
      expect(String(lookup(en, key)).trim(), key).not.toBe("");
    }
  });

  test.each(others)("%s has exactly the English keys", (_lang, table) => {
    expect(keys(table).sort()).toEqual(keys(en).sort());
  });

  test.each(others)("%s has no empty text and the same placeholders", (_lang, table) => {
    for (const key of keys(en)) {
      const text = lookup(table, key);
      expect(typeof text, key).toBe("string");
      expect((text as string).trim(), key).not.toBe("");
      expect(placeholders(text as string), key).toEqual(placeholders(lookup(en, key) as string));
    }
  });
});

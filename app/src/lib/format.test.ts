import { expect, test } from "vitest";
import { initI18nOnce, LANGUAGES } from "@/i18n";
import { setFixedClock } from "./clock";
import { formatDateTime, formatDuration, formatRelative, formatTime } from "./format";

test("relative times and durations read naturally in each language", async () => {
  // Explicitly started here: the strings come from our own translation keys.
  await initI18nOnce("en");
  setFixedClock(new Date("2026-10-12T09:00:00Z"));
  expect(formatRelative("2026-10-12T08:57:00Z", "en")).toBe("3 minutes ago");
  expect(formatRelative("2026-10-12T08:59:00Z", "en")).toBe("1 minute ago");
  expect(formatRelative("2026-10-12T08:59:50Z", "en")).toBe("Just now");
  expect(formatRelative("2026-10-12T06:00:00Z", "en")).toBe("3 hours ago");
  expect(formatRelative("2026-10-12T08:57:00Z", "zh-CN")).toBe("3 分钟前");
  expect(formatDuration("2026-10-12T08:57:37Z", "2026-10-12T09:00:00Z", "en")).toBe("2 min 23 s");
  expect(formatDuration("2026-10-12T08:57:37Z", "2026-10-12T09:00:00Z", "zh-CN")).toBe("2 分 23 秒");
  expect(formatDuration("2026-10-12T08:59:50Z", "2026-10-12T09:00:00Z", "en")).toBe("10 s");
  expect(formatDuration("2026-10-12T07:00:00Z", null, "en")).toBe("2 h 0 min");
});

test("the language asked for decides, not the one the interface is in", async () => {
  await initI18nOnce("en");
  setFixedClock(new Date("2026-10-12T09:00:00Z"));
  expect(formatRelative("2026-10-12T08:57:00Z", "ja")).toBe("3 分前");
  expect(formatRelative("2026-10-12T08:57:00Z", "ko")).toBe("3분 전");
  expect(formatRelative("2026-10-12T08:57:00Z", "zh-TW")).toBe("3 分鐘前");
  expect(formatDuration("2026-10-12T07:00:00Z", null, "ko")).toBe("2시간 0분");
});

test("hours and days count up from one to many, and a week later the date is shown", async () => {
  await initI18nOnce("en");
  setFixedClock(new Date("2026-10-12T09:00:00Z"));
  expect(formatRelative("2026-10-12T08:00:00Z", "en")).toBe("1 hour ago");
  expect(formatRelative("2026-10-12T08:01:00Z", "en")).toBe("59 minutes ago");
  expect(formatRelative("2026-10-11T09:00:00Z", "en")).toBe("1 day ago");
  expect(formatRelative("2026-10-09T09:00:00Z", "en")).toBe("3 days ago");
  expect(formatRelative("2026-10-05T09:00:00Z", "en")).toBe("7 days ago");
  // More than seven days ago: the date, because "9 days ago" no longer tells when.
  expect(formatRelative("2026-10-05T08:59:00Z", "en")).toBe(formatDateTime("2026-10-05T08:59:00Z", "en"));
  expect(formatRelative("2026-09-01T09:00:00Z", "en")).toBe(formatDateTime("2026-09-01T09:00:00Z", "en"));
});

test("a time that is not yet reached reads as just now, and a duration never goes below zero", async () => {
  await initI18nOnce("en");
  setFixedClock(new Date("2026-10-12T09:00:00Z"));
  expect(formatRelative("2026-10-12T09:00:30Z", "en")).toBe("Just now");
  expect(formatDuration("2026-10-12T09:00:30Z", "2026-10-12T09:00:00Z", "en")).toBe("0 s");
});

test("an open duration runs up to now", async () => {
  await initI18nOnce("en");
  setFixedClock(new Date("2026-10-12T09:00:00Z"));
  expect(formatDuration("2026-10-12T08:59:00Z", null, "en")).toBe("1 min 0 s");
  setFixedClock(new Date("2026-10-12T09:00:05Z"));
  expect(formatDuration("2026-10-12T08:59:00Z", null, "en")).toBe("1 min 5 s");
});

test("dates and times are formatted for the language, with the hour and minute in the time", () => {
  for (const lang of LANGUAGES) {
    expect(formatTime("2026-10-12T09:02:00Z", lang), lang).toMatch(/\d{1,2}:\d{2}/);
    const dateTime = formatDateTime("2026-10-12T09:02:00Z", lang);
    expect(dateTime, lang).toMatch(/2026|26/);
    expect(dateTime, lang).toMatch(/\d{1,2}:\d{2}/);
  }
});

test("a time that cannot be read says unknown instead of breaking the page", async () => {
  await initI18nOnce("en");
  setFixedClock(new Date("2026-10-12T09:00:00Z"));
  expect(formatRelative("not a time", "en")).toBe("Unknown");
  expect(formatDateTime("", "en")).toBe("Unknown");
  expect(formatTime("not a time", "en")).toBe("Unknown");
  expect(formatDuration("not a time", null, "en")).toBe("Unknown");
  expect(formatDuration("2026-10-12T08:00:00Z", "not a time", "en")).toBe("Unknown");
});

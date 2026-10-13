import i18n, { type Lang } from "@/i18n";
import { now } from "./clock";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
/** Older than this, "9 days ago" no longer says when: the date is shown. */
const RELATIVE_LIMIT = 7 * DAY;

const formatters = new Map<string, Intl.DateTimeFormat>();

/** `Intl.DateTimeFormat` for display only (tests do not compare its exact output); built once per language and style. */
function dateFormat(lang: Lang, style: "date-time" | "time"): Intl.DateTimeFormat {
  const id = `${lang}/${style}`;
  let format = formatters.get(id);
  if (format === undefined) {
    format = new Intl.DateTimeFormat(
      lang,
      style === "time" ? { hour: "2-digit", minute: "2-digit" } : { dateStyle: "medium", timeStyle: "short" },
    );
    formatters.set(id, format);
  }
  return format;
}

/** The date and time of `iso` in the user's time zone. */
export function formatDateTime(iso: string, lang: Lang): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? i18n.getFixedT(lang)("common.unknown") : dateFormat(lang, "date-time").format(at);
}

/** Hour and minute of `iso` in the user's time zone ("retrying at 17:02"). */
export function formatTime(iso: string, lang: Lang): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? i18n.getFixedT(lang)("common.unknown") : dateFormat(lang, "time").format(at);
}

/**
 * How long ago `iso` was: "Just now", then minutes, hours and days, and the date after a week.
 * Worded by our own translation keys (with i18next plurals) in the language asked for, not the
 * one the interface is in. A time that is not yet reached reads as "just now".
 */
export function formatRelative(iso: string, lang: Lang): string {
  const t = i18n.getFixedT(lang);
  const at = new Date(iso).getTime();
  if (Number.isNaN(at)) return t("common.unknown");
  const elapsed = now().getTime() - at;
  if (elapsed < MINUTE) return t("format.just_now");
  if (elapsed < HOUR) return t("format.minutes_ago", { count: Math.floor(elapsed / MINUTE) });
  if (elapsed < DAY) return t("format.hours_ago", { count: Math.floor(elapsed / HOUR) });
  if (elapsed <= RELATIVE_LIMIT) return t("format.days_ago", { count: Math.floor(elapsed / DAY) });
  return formatDateTime(iso, lang);
}

/**
 * The time from `fromIso` to `toIso`, or to now while the run is still going (`null`): seconds,
 * minutes and seconds, or hours and minutes.
 */
export function formatDuration(fromIso: string, toIso: string | null, lang: Lang): string {
  const t = i18n.getFixedT(lang);
  const start = new Date(fromIso).getTime();
  const end = toIso === null ? now().getTime() : new Date(toIso).getTime();
  if (Number.isNaN(start) || Number.isNaN(end)) return t("common.unknown");
  const seconds = Math.max(0, Math.floor((end - start) / 1000));
  if (seconds < 60) return t("format.duration_s", { s: seconds });
  if (seconds < 3600) return t("format.duration_ms", { m: Math.floor(seconds / 60), s: seconds % 60 });
  return t("format.duration_hm", { h: Math.floor(seconds / 3600), m: Math.floor((seconds % 3600) / 60) });
}

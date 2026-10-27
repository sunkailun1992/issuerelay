import { cleanDisplayText } from "./clean";
import type { Effort, ExecutorKind, NotificationSettings, PermissionProfile, Settings, TriggerNames } from "./types";

/** The settings that are counts: seconds, minutes, runs, days and megabytes. */
export type CountField =
  | "poll_interval_secs"
  | "max_concurrent"
  | "run_timeout_mins"
  | "stall_timeout_mins"
  | "log_retention_days"
  | "log_max_total_mb";

/** How low each count may go: `Settings::validate` in crates/core/src/config.rs. */
export const MINIMUM: Record<CountField, number> = {
  poll_interval_secs: 5,
  max_concurrent: 1,
  run_timeout_mins: 1,
  stall_timeout_mins: 1,
  log_retention_days: 1,
  log_max_total_mb: 1,
};

/**
 * What the settings page edits. The counts are text, so that a number that is half typed (or
 * emptied to be typed again) is kept as it is; they become numbers when the page is saved. What the
 * page does not edit (the monitoring switch, whether the guide was finished) is not here: it
 * comes from the settings as they are when the page is saved.
 */
export interface Draft extends Record<CountField, string> {
  language: string;
  comment_language: string;
  default_executor: ExecutorKind;
  default_permission: PermissionProfile;
  /** `null`: no default, each executor uses its own setting. */
  default_effort: Effort | null;
  prompt_template: string;
  notifications: NotificationSettings;
  auto_update: boolean;
  trigger: TriggerNames;
  /** The user confirmed what full access means (it is saved with the page). */
  full_access_acknowledged: boolean;
  /** Whether the app keeps its icon in the macOS Dock; only the macOS page offers it, and on Windows it stays as it is saved. */
  show_in_dock: boolean;
  /** An issue that comes back continues the executor session of its earlier run. */
  resume_sessions: boolean;
}

export const COUNT_FIELDS = Object.keys(MINIMUM) as CountField[];

/** A copy of what `settings` holds for the page to edit. */
export function draftOf(settings: Settings): Draft {
  return {
    language: settings.language,
    comment_language: settings.comment_language,
    poll_interval_secs: String(settings.poll_interval_secs),
    max_concurrent: String(settings.max_concurrent),
    run_timeout_mins: String(settings.run_timeout_mins),
    stall_timeout_mins: String(settings.stall_timeout_mins),
    log_retention_days: String(settings.log_retention_days),
    log_max_total_mb: String(settings.log_max_total_mb),
    default_executor: settings.default_executor,
    default_permission: settings.default_permission,
    default_effort: settings.default_effort ?? null,
    prompt_template: settings.prompt_template,
    notifications: { ...settings.notifications },
    auto_update: settings.auto_update,
    trigger: { ...settings.trigger },
    full_access_acknowledged: settings.full_access_acknowledged,
    show_in_dock: settings.show_in_dock,
    resume_sessions: settings.resume_sessions,
  };
}

/**
 * What is wrong with a value, as a code for the page to word. `same_as_label`: the plan label is the
 * trigger label (core's `Settings::validate`), which would leave no way to ask for a plan.
 */
export type Problem = { kind: "min"; min: number } | { kind: "whole" } | { kind: "name" } | { kind: "same_as_label" };

/**
 * Whether `text` can be a count of at least `min`. An empty field is below any limit; anything
 * that is not a whole number, or too big to be kept exactly, is not a whole number.
 */
export function countProblem(text: string, min: number): Problem | null {
  const trimmed = text.trim();
  if (trimmed === "") return { kind: "min", min };
  if (!/^-?\d+$/.test(trimmed)) return { kind: "whole" };
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value)) return { kind: "whole" };
  return value < min ? { kind: "min", min } : null;
}

/**
 * Whether `name` can be the name of the label or of a status: core's `is_plain_name`, one
 * non-empty line without quotes. The names go inside quotes in the prompt's fixed rules, so
 * control characters and invisible ones are out as well; `cleanDisplayText` changes a text exactly
 * when it holds one of them (or an escape sequence, which starts with one).
 */
export function nameProblem(name: string): Problem | null {
  const plain = name.trim() !== "" && !name.includes('"') && cleanDisplayText(name) === name;
  return plain ? null : { kind: "name" };
}

/** Whether two names of Linear's are the same, as core's `same_name`: without the blanks around them, ASCII case aside. */
export function sameName(a: string, b: string): boolean {
  const fold = (name: string) => name.trim().replace(/[A-Z]/g, (c) => c.toLowerCase());
  return fold(a) === fold(b);
}

/** The problems of a draft by the field they belong to (`trigger.label` for the label); none for a good draft. */
export function problemsOf(draft: Draft): Partial<Record<string, Problem>> {
  const problems: Partial<Record<string, Problem>> = {};
  for (const field of COUNT_FIELDS) {
    const problem = countProblem(draft[field], MINIMUM[field]);
    if (problem !== null) problems[field] = problem;
  }
  for (const [field, name] of Object.entries(draft.trigger)) {
    const problem = nameProblem(name);
    if (problem !== null) problems[`trigger.${field}`] = problem;
  }
  if (problems["trigger.plan_label"] === undefined && sameName(draft.trigger.plan_label, draft.trigger.label)) {
    problems["trigger.plan_label"] = { kind: "same_as_label" };
  }
  return problems;
}

/**
 * The settings to save: `base` (the settings as they are now) with the fields of the draft. Names
 * lose the blanks around them (a status is matched by its name); the template stays as it was typed.
 * No default effort is left out, as the engine writes it. `null` when a count is not a number:
 * `problemsOf` says which.
 */
export function settingsOf(base: Settings, draft: Draft): Settings | null {
  const counts: Partial<Record<CountField, number>> = {};
  for (const field of COUNT_FIELDS) {
    if (countProblem(draft[field], MINIMUM[field]) !== null) return null;
    counts[field] = Number(draft[field].trim());
  }
  const settings: Settings = {
    ...base,
    ...counts,
    language: draft.language,
    comment_language: draft.comment_language,
    default_executor: draft.default_executor,
    default_permission: draft.default_permission,
    prompt_template: draft.prompt_template,
    notifications: { ...draft.notifications },
    auto_update: draft.auto_update,
    trigger: {
      label: draft.trigger.label.trim(),
      todo: draft.trigger.todo.trim(),
      in_progress: draft.trigger.in_progress.trim(),
      in_testing: draft.trigger.in_testing.trim(),
      done: draft.trigger.done.trim(),
      plan_label: draft.trigger.plan_label.trim(),
    },
    full_access_acknowledged: draft.full_access_acknowledged,
    show_in_dock: draft.show_in_dock,
    resume_sessions: draft.resume_sessions,
  };
  if (draft.default_effort === null) delete settings.default_effort;
  else settings.default_effort = draft.default_effort;
  return settings;
}

/** Whether two drafts hold the same everything. */
export function sameDraft(a: Draft, b: Draft): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The edits that `saved` did not already carry, so that what was typed while a save ran is not lost with it. */
export function unsaved(edits: Partial<Draft>, saved: Draft): Partial<Draft> {
  return Object.fromEntries(
    Object.entries(edits).filter(([field, value]) => JSON.stringify(value) !== JSON.stringify(saved[field as keyof Draft])),
  );
}

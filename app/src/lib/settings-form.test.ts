import { expect, test } from "vitest";
import { DEFAULT_SETTINGS } from "./mock-fixtures";
import { countProblem, draftOf, MINIMUM, nameProblem, problemsOf, sameDraft, settingsOf, unsaved } from "./settings-form";
import type { Settings } from "./types";

const ESC = String.fromCharCode(27);
const ZERO_WIDTH_SPACE = String.fromCharCode(0x200b);
const RIGHT_TO_LEFT_OVERRIDE = String.fromCharCode(0x202e);

test("the form works on text for the numbers, so that a half-typed number is not lost", () => {
  const draft = draftOf(DEFAULT_SETTINGS);
  expect(draft.poll_interval_secs).toBe("30");
  expect(draft.max_concurrent).toBe("2");
  expect(draft.run_timeout_mins).toBe("60");
  expect(draft.stall_timeout_mins).toBe("15");
  expect(draft.log_retention_days).toBe("30");
  expect(draft.log_max_total_mb).toBe("1024");
});

test("the form edits a copy: the settings it was made from are not touched", () => {
  const settings = structuredClone(DEFAULT_SETTINGS);
  const draft = draftOf(settings);
  draft.trigger.label = "changed";
  draft.notifications.failed = false;
  expect(settings.trigger.label).toBe("ai-ready");
  expect(settings.notifications.failed).toBe(true);
});

test("a draft that was not changed gives the settings it came from, whatever the engine sent", () => {
  const settings: Settings = { ...DEFAULT_SETTINGS, paused: true, onboarding_completed: true, comment_language: "ja" };
  expect(settingsOf(settings, draftOf(settings))).toEqual(settings);
});

test("what the form does not edit comes from the settings it is given, not from when the form was opened", () => {
  const opened: Settings = { ...DEFAULT_SETTINGS, paused: false, onboarding_completed: true };
  const draft = draftOf(opened);
  draft.poll_interval_secs = "60";
  // Monitoring was paused from the overview after the form was opened: saving must not resume it.
  const latest: Settings = { ...opened, paused: true };
  expect(settingsOf(latest, draft)).toEqual({ ...latest, poll_interval_secs: 60 });
});

test("every field the form edits reaches the settings", () => {
  const draft = draftOf(DEFAULT_SETTINGS);
  Object.assign(draft, {
    language: "ja",
    comment_language: "ko",
    poll_interval_secs: "45",
    max_concurrent: "3",
    run_timeout_mins: "90",
    stall_timeout_mins: "20",
    log_retention_days: "14",
    log_max_total_mb: "512",
    default_executor: "codex",
    default_permission: "auto",
    default_effort: "max",
    prompt_template: "Push to the test branch.",
    auto_update: false,
    full_access_acknowledged: true,
    show_in_dock: false,
    notifications: { run_started: false, awaiting_acceptance: false, needs_info: false, failed: false },
    trigger: { label: "ready", todo: "Backlog", in_progress: "Doing", in_testing: "Review", done: "Shipped" },
  });
  expect(settingsOf(DEFAULT_SETTINGS, draft)).toEqual({
    ...DEFAULT_SETTINGS,
    language: "ja",
    comment_language: "ko",
    poll_interval_secs: 45,
    max_concurrent: 3,
    run_timeout_mins: 90,
    stall_timeout_mins: 20,
    log_retention_days: 14,
    log_max_total_mb: 512,
    default_executor: "codex",
    default_permission: "auto",
    default_effort: "max",
    prompt_template: "Push to the test branch.",
    auto_update: false,
    full_access_acknowledged: true,
    show_in_dock: false,
    notifications: { run_started: false, awaiting_acceptance: false, needs_info: false, failed: false },
    trigger: { label: "ready", todo: "Backlog", in_progress: "Doing", in_testing: "Review", done: "Shipped" },
  });
});

test("the default effort comes from the settings and can be cleared, which leaves it out", () => {
  const withEffort = { ...DEFAULT_SETTINGS, default_effort: "high" as const };
  const draft = draftOf(withEffort);
  expect(draft.default_effort).toBe("high");
  expect(settingsOf(withEffort, draft)).toEqual(withEffort);
  draft.default_effort = null;
  const cleared = settingsOf(withEffort, draft)!;
  expect("default_effort" in cleared).toBe(false);
  expect(draftOf(DEFAULT_SETTINGS).default_effort).toBeNull();
  expect("default_effort" in settingsOf(DEFAULT_SETTINGS, draftOf(DEFAULT_SETTINGS))!).toBe(false);
});

test("the Dock switch is part of the draft: it comes from the settings and a draft that left it alone gives it back", () => {
  const off: Settings = { ...DEFAULT_SETTINGS, show_in_dock: false };
  expect(draftOf(DEFAULT_SETTINGS).show_in_dock).toBe(true);
  expect(draftOf(off).show_in_dock).toBe(false);
  expect(settingsOf(off, draftOf(off))).toEqual(off);
  const draft = draftOf(off);
  draft.show_in_dock = true;
  expect(settingsOf(off, draft)?.show_in_dock).toBe(true);
  expect(sameDraft(draft, draftOf(off))).toBe(false);
});

test("names are saved without the blanks around them, the template as it was typed", () => {
  const draft = draftOf(DEFAULT_SETTINGS);
  draft.trigger = { label: " a ", todo: "  Todo  ", in_progress: "\tc", in_testing: "d  ", done: " e " };
  draft.prompt_template = "  keep my indentation\n";
  const settings = settingsOf(DEFAULT_SETTINGS, draft);
  expect(settings?.trigger).toEqual({ label: "a", todo: "Todo", in_progress: "c", in_testing: "d", done: "e" });
  expect(settings?.prompt_template).toBe("  keep my indentation\n");
});

test("a draft with a number that is not one has no settings", () => {
  const draft = draftOf(DEFAULT_SETTINGS);
  draft.max_concurrent = "";
  expect(settingsOf(DEFAULT_SETTINGS, draft)).toBeNull();
});

test("the lower limits are the engine's", () => {
  // `Settings::validate` in crates/core/src/config.rs.
  expect(MINIMUM).toEqual({
    poll_interval_secs: 5,
    max_concurrent: 1,
    run_timeout_mins: 1,
    stall_timeout_mins: 1,
    log_retention_days: 1,
    log_max_total_mb: 1,
  });
});

test.each([
  ["5", 5, null],
  ["4", 5, { kind: "min", min: 5 }],
  ["2", 5, { kind: "min", min: 5 }],
  ["0", 1, { kind: "min", min: 1 }],
  ["-3", 1, { kind: "min", min: 1 }],
  ["", 5, { kind: "min", min: 5 }],
  ["   ", 5, { kind: "min", min: 5 }],
  ["5.5", 5, { kind: "whole" }],
  ["abc", 1, { kind: "whole" }],
  ["1e3", 1, { kind: "whole" }],
  ["+7", 1, { kind: "whole" }],
  [" 12 ", 1, null],
  ["007", 1, null],
  ["9007199254740991", 1, null],
  ["9007199254740993", 1, { kind: "whole" }],
  ["99999999999999999999", 1, { kind: "whole" }],
])("the number %j with a lower limit of %i is %j", (text, min, problem) => {
  expect(countProblem(text, min)).toEqual(problem);
});

test.each([
  ["Todo", null],
  ["In Progress", null],
  ["ai-ready", null],
  ["  Todo  ", null],
  ["Fertig (erledigt)", null],
  ["完成", null],
  ["", { kind: "name" }],
  ["   ", { kind: "name" }],
  ['say "yes"', { kind: "name" }],
  ['"', { kind: "name" }],
  ["two\nlines", { kind: "name" }],
  ["a\tb", { kind: "name" }],
  [`red${ESC}[31m`, { kind: "name" }],
  [`Do${ZERO_WIDTH_SPACE}ne`, { kind: "name" }],
  [`Done${RIGHT_TO_LEFT_OVERRIDE}`, { kind: "name" }],
])("the name %j is %j", (name, problem) => {
  expect(nameProblem(name)).toEqual(problem);
});

test("a draft from good settings has no problem", () => {
  expect(problemsOf(draftOf(DEFAULT_SETTINGS))).toEqual({});
});

test("every number and name that is no good is reported under the field it belongs to", () => {
  const draft = draftOf(DEFAULT_SETTINGS);
  draft.poll_interval_secs = "2";
  draft.log_max_total_mb = "x";
  draft.trigger.done = "";
  draft.trigger.label = 'a"b';
  expect(problemsOf(draft)).toEqual({
    poll_interval_secs: { kind: "min", min: 5 },
    log_max_total_mb: { kind: "whole" },
    "trigger.done": { kind: "name" },
    "trigger.label": { kind: "name" },
  });
});

test("two drafts are the same when every field is", () => {
  const a = draftOf(DEFAULT_SETTINGS);
  expect(sameDraft(a, draftOf(DEFAULT_SETTINGS))).toBe(true);
  const changed = draftOf(DEFAULT_SETTINGS);
  changed.notifications = { ...changed.notifications, failed: false };
  expect(sameDraft(a, changed)).toBe(false);
  const named = draftOf(DEFAULT_SETTINGS);
  named.trigger = { ...named.trigger, todo: "Backlog" };
  expect(sameDraft(a, named)).toBe(false);
  const typed = draftOf(DEFAULT_SETTINGS);
  typed.poll_interval_secs = "31";
  expect(sameDraft(a, typed)).toBe(false);
});

test("what a save already carried is no longer an edit, and what was typed meanwhile still is", () => {
  const saved = draftOf(DEFAULT_SETTINGS);
  saved.poll_interval_secs = "60";
  expect(unsaved({ poll_interval_secs: "60", max_concurrent: "5" }, saved)).toEqual({ max_concurrent: "5" });
  // Objects are compared by what they hold, not by who they are.
  saved.notifications = { ...saved.notifications, failed: false };
  expect(unsaved({ notifications: { ...saved.notifications } }, saved)).toEqual({});
  expect(unsaved({ notifications: { ...saved.notifications, needs_info: false } }, saved)).toEqual({
    notifications: { ...saved.notifications, needs_info: false },
  });
  expect(unsaved({}, saved)).toEqual({});
});

import { MOCK_NOW } from "./clock";
import shellLine from "./shell-line.json";
import triggerDefaults from "./trigger-defaults.json";
import type { AppInfo, Binding, DoctorReport, HeldJob, Issue, IssueStatus, Job, Project, Settings, UpdateStatus, Viewer } from "./types";

/** `MOCK_NOW` minus `minutes`, as the engine sends times. */
export function ago(minutes: number): string {
  return new Date(MOCK_NOW.getTime() - minutes * 60_000).toISOString();
}

export const DEMO_VIEWER: Viewer = { id: "user-1", name: "Jane Doe" };

export const DEMO_APP_INFO: AppInfo = {
  version: "0.1.0",
  cli_path: "/Applications/IssueRelay.app/Contents/MacOS/issuerelay",
  platform: "macos",
  system_locale: "en-US",
};

/** The line the panel shows when ~/.local/bin is not on the login shell's PATH: core's `cli_path::SHELL_LINE`. */
export const DEMO_SHELL_LINE: string = shellLine.shell_line;

/**
 * What the app's update check can have found, by the name the browser preview asks for with
 * `?update=` (the layout checks of the banner and of the settings row). The reason of the failed
 * install is as long as a real one gets (about 200 characters): a sentence, a path whose name cannot
 * break by itself, and the system's note.
 */
export const DEMO_UPDATES: Record<string, UpdateStatus> = {
  available: { kind: "available", version: "0.2.0" },
  "install-failed": {
    kind: "failed",
    code: "install_failed",
    detail:
      "failed to install the update and to restore the previous app, which was left at /Applications/IssueRelay_previous_version_0_1_0_kept_because_it_could_not_be_moved_back_to_its_place.app (permission denied, os error 13)",
  },
};

export const DEMO_PROJECTS: Project[] = [
  { id: "proj-1", name: "Demo Project", team_ids: ["team-1"] },
  { id: "proj-2", name: "Demo Mobile", team_ids: ["team-1"] },
  { id: "proj-3", name: "Docs Site", team_ids: ["team-2"] },
];

export const DEMO_BINDINGS: Binding[] = [
  { id: "proj-1", project_id: "proj-1", project_name: "Demo Project", directory: "~/code/demo-app", permission: "full" },
  {
    id: "proj-2",
    project_id: "proj-2",
    project_name: "Demo Mobile",
    directory: "~/code/demo-mobile",
    executor: "codex",
    permission: "auto",
    model: "gpt-5-codex",
    lean: false,
    effort: "high",
  },
];

/** `config::Settings::default()`. */
export const DEFAULT_SETTINGS: Settings = {
  language: "system",
  poll_interval_secs: 30,
  max_concurrent: 2,
  run_timeout_mins: 60,
  stall_timeout_mins: 15,
  log_retention_days: 30,
  log_max_total_mb: 1024,
  default_executor: "claude",
  default_permission: "full",
  prompt_template: "",
  comment_language: "follow-ui",
  paused: false,
  trigger: triggerDefaults,
  notifications: { run_started: true, awaiting_acceptance: true, needs_info: true, failed: true },
  auto_update: true,
  onboarding_completed: false,
  full_access_acknowledged: false,
  show_in_dock: true,
};

/** A demo job: what every demo job shares, then `fields`. */
function job(fields: Pick<Job, "id" | "issue_identifier" | "title" | "state"> & Partial<Job>): Job {
  return {
    issue_uuid: `issue-${fields.issue_identifier.replace("DEMO-", "")}`,
    binding_id: "proj-1",
    directory: "~/code/demo-app",
    executor: "claude",
    reason: null,
    priority: 2,
    issue_created_at: ago(24 * 60),
    discovered_updated_at: ago(12 * 60),
    verified_updated_at: null,
    created_at: ago(12 * 60),
    started_at: null,
    finished_at: null,
    log_path: null,
    summary: null,
    ...fields,
  };
}

/**
 * One job in every state, newest first, plus an older failed run of DEMO-12 (id 2): only the
 * newest run of an issue may run again, and an old failure does not raise the overview's alert.
 */
export const DEMO_JOBS: Job[] = [
  job({ id: 12, issue_identifier: "DEMO-12", title: "Add a health check endpoint", state: "running", started_at: "2026-10-12T08:57:37Z", log_path: "logs/12.jsonl" }),
  job({ id: 11, issue_identifier: "DEMO-11", title: "Translate the settings page", state: "queued", binding_id: "proj-2", directory: "~/code/demo-mobile", executor: "codex" }),
  job({ id: 10, issue_identifier: "DEMO-10", title: "Add hello.txt", state: "awaiting_acceptance", started_at: ago(50), finished_at: ago(20), verified_updated_at: ago(20), log_path: "logs/10.jsonl", summary: "Added hello.txt, pushed it to the test branch and read the deployment back" }),
  job({ id: 9, issue_identifier: "DEMO-9", title: "Clarify the export format", state: "needs_info", started_at: ago(70), finished_at: ago(60), summary: "Asked on the issue which date format the export should use" }),
  job({ id: 8, issue_identifier: "DEMO-8", title: "Speed up the search index", state: "failed", reason: "run exceeded the time limit", started_at: ago(130), finished_at: ago(70) }),
  job({ id: 7, issue_identifier: "DEMO-7", title: "Fix the login redirect", state: "done", started_at: ago(240), finished_at: ago(200), summary: "Fixed the redirect after login and added a test" }),
  job({ id: 6, issue_identifier: "DEMO-6", title: "Remove the old banner", state: "canceled", reason: "stopped by the user", started_at: ago(300), finished_at: ago(290) }),
  job({ id: 5, issue_identifier: "DEMO-5", title: "Write the docs landing page", state: "waiting_binding", reason: "project is not bound to a local directory", binding_id: null, directory: null, executor: null }),
  job({ id: 2, issue_identifier: "DEMO-12", title: "Add a health check endpoint", state: "failed", reason: "executor exited with code 1", started_at: ago(2 * 24 * 60), finished_at: ago(2 * 24 * 60 - 5) }),
];

/** DEMO-11's start failed once; it is tried again two minutes after `MOCK_NOW`. */
export const DEMO_HELD: HeldJob[] = [{ job_id: 11, retry_at: "2026-10-12T09:02:00Z" }];

/**
 * A labeled issue. Nothing changed in Linear since the demo's runs were recorded: `updated_at` is the
 * moment their `discovered_updated_at` has, so the next check does not start a finished one again.
 */
function issue(n: number, title: string, state: string, project: Project): Issue {
  return {
    uuid: `issue-${n}`,
    identifier: `DEMO-${n}`,
    title,
    url: `https://linear.app/demo/issue/DEMO-${n}`,
    state,
    labels: ["ai-ready"],
    project_id: project.id,
    project_name: project.name,
    assignee_id: "user-1",
    parent_identifier: null,
    priority: 2,
    created_at: ago(24 * 60),
    updated_at: ago(12 * 60),
  };
}

const [demoProject, demoMobile, docsSite] = DEMO_PROJECTS;

/**
 * The issue list (the issues with the label; DEMO-9 lost it when the AI asked back) and the engine's
 * verdicts. DEMO-6's run was stopped and nothing changed since, so the next check does not pick it up
 * (`picked_up: false`); DEMO-11's run is queued, DEMO-3 has none: both are picked up.
 */
export const DEMO_ISSUES: { issue: Issue; status: IssueStatus }[] = [
  { issue: issue(12, "Add a health check endpoint", "In Progress", demoProject), status: { kind: "not_todo", state: "In Progress" } },
  { issue: issue(11, "Translate the settings page", "Todo", demoMobile), status: { kind: "eligible", picked_up: true } },
  { issue: issue(10, "Add hello.txt", "In Testing", demoProject), status: { kind: "not_todo", state: "In Testing" } },
  { issue: issue(8, "Speed up the search index", "In Progress", demoProject), status: { kind: "not_todo", state: "In Progress" } },
  { issue: issue(7, "Fix the login redirect", "Done", demoProject), status: { kind: "not_todo", state: "Done" } },
  { issue: issue(6, "Remove the old banner", "Todo", demoProject), status: { kind: "eligible", picked_up: false } },
  { issue: issue(5, "Write the docs landing page", "Todo", docsSite), status: { kind: "waiting", reason: { code: "project_not_bound" } } },
  { issue: issue(4, "Migrate the build to the new runner", "Backlog", demoProject), status: { kind: "not_todo", state: "Backlog" } },
  { issue: issue(3, "Update the README badges", "Todo", demoProject), status: { kind: "eligible", picked_up: true } },
];

export const DEMO_DOCTOR: DoctorReport = {
  viewer: DEMO_VIEWER,
  linear_error: null,
  executors: [
    {
      kind: "claude",
      program: "~/.local/bin/claude",
      version: "2.1.285 (Claude Code)",
      logged_in: true,
      linear_mcp: true,
      problems: [],
      lean_linear_mcp: true,
      lean_logged_in: true,
      lean_problems: [],
    },
    {
      kind: "codex",
      program: "~/.local/bin/codex",
      version: "codex-cli 0.159.2",
      logged_in: false,
      linear_mcp: true,
      problems: ["codex is not logged in"],
      lean_linear_mcp: false,
      lean_logged_in: false,
      lean_problems: ["lean runs use the Linear MCP server named `linear` in codex (register it with: codex mcp add linear --url https://mcp.linear.app/mcp)"],
    },
  ],
  workspace: { label_ok: true, missing_states: [], teams_checked: ["Demo"] },
  bindings: [
    { binding_id: "proj-1", project_name: "Demo Project", directory_exists: true, executor: "claude", executor_ready: true, lean: "on", effort: null },
    { binding_id: "proj-2", project_name: "Demo Mobile", directory_exists: true, executor: "codex", executor_ready: false, lean: "off", effort: "high" },
  ],
  env: {
    path: "~/.local/bin:/opt/homebrew/bin:/usr/bin:/bin",
    path_source: "login-shell",
    proxy: [["HTTPS_PROXY", "http://127.0.0.1:7890"]],
    proxy_source: "system-settings",
    problems: [],
  },
};

/** Before the first login: no user, no workspace check, no bindings. */
export const FRESH_DOCTOR: DoctorReport = {
  ...DEMO_DOCTOR,
  viewer: null,
  linear_error: "not connected to Linear: log in first",
  workspace: null,
  bindings: [],
};

const CLAUDE_SESSION = [
  JSON.stringify({ type: "system", subtype: "init", model: "claude-sonnet-5-5", session_id: "demo-session" }),
  JSON.stringify({ type: "assistant", message: { content: [{ type: "text", text: "I will add the endpoint and a test for it." }] } }),
  JSON.stringify({ type: "assistant", message: { content: [{ type: "tool_use", id: "tool-1", name: "Bash", input: { command: "cargo test" } }] } }),
  JSON.stringify({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "tool-1", content: "test result: ok. 12 passed", is_error: false }] } }),
];

/** Run logs by job id; the other jobs' logs are gone (removed by the cleanup) and read as empty. */
export const DEMO_LOGS: Record<number, string[]> = {
  12: [...CLAUDE_SESSION, JSON.stringify({ stream: "stderr", text: "warning: unused import" })],
  10: [...CLAUDE_SESSION, JSON.stringify({ type: "result", result: "Added hello.txt and pushed it to the test branch.", is_error: false })],
};

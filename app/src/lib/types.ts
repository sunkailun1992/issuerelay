export type JobState =
  | "waiting_binding"
  | "queued"
  | "running"
  | "awaiting_acceptance"
  | "needs_info"
  | "done"
  | "failed"
  | "canceled";
export const JOB_STATES: JobState[] = [
  "waiting_binding",
  "queued",
  "running",
  "awaiting_acceptance",
  "needs_info",
  "done",
  "failed",
  "canceled",
];
export type ExecutorKind = "claude" | "codex";
export type PermissionProfile = "full" | "auto" | "personal";
/** How hard the executor reasons (`config::Effort`). */
export type Effort = "low" | "medium" | "high" | "xhigh" | "max";
/** The reasoning effort levels, lowest first. */
export const EFFORTS: readonly Effort[] = ["low", "medium", "high", "xhigh", "max"];
/**
 * Whether a binding's runs are lean (`config::LeanStatus`): only `on` is; the other three run with
 * the user's whole configuration (lean runs turned off, personal settings, or the home folder).
 */
export type LeanStatus = "on" | "off" | "personal" | "home_directory";

export interface Viewer { id: string; name: string }

export interface Issue {
  uuid: string;
  identifier: string;
  title: string;
  url: string;
  state: string;
  labels: string[];
  project_id: string | null;
  project_name: string | null;
  assignee_id: string | null;
  parent_identifier: string | null;
  priority: number;
  created_at: string;
  updated_at: string;
}

export interface Job {
  id: number;
  issue_uuid: string;
  issue_identifier: string;
  title: string;
  binding_id: string | null;
  directory: string | null;
  executor: ExecutorKind | null;
  state: JobState;
  reason: string | null;
  priority: number;
  issue_created_at: string;
  discovered_updated_at: string;
  verified_updated_at: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  log_path: string | null;
  summary: string | null;
}

/** A queued job passed over after a failed start, and when it is tried again. */
export interface HeldJob { job_id: number; retry_at: string }

export interface StatusSnapshot {
  paused: boolean;
  linear_ok: boolean;
  viewer: Viewer | null;
  last_poll_at: string | null;
  last_error: string | null;
  running: number;
  queued: number;
  waiting: number;
  draining: boolean;
  held: HeldJob[];
}

export type AuthMethod = "oauth" | "api_key";
export type LinearSetup =
  | { kind: "connected"; method: AuthMethod }
  | { kind: "not_configured" }
  | { kind: "interface_changed"; message: string }
  | { kind: "connect_failed"; message: string }
  /** The first connection is still under way (the host no longer waits for it). */
  | { kind: "connecting" };
export interface StatusView { status: StatusSnapshot; linear: LinearSetup }

/** Why an eligible-looking issue waits, as a code the panel translates. */
export type WaitReason =
  | { code: "project_not_bound" }
  | { code: "directory_missing"; directory: string }
  | { code: "executor_not_ready"; executor: ExecutorKind }
  /** Ready for usual runs, but the binding's runs are lean and the lean checks fail. */
  | { code: "lean_not_ready"; executor: ExecutorKind };
export type IssueStatus =
  /**
   * It meets every condition. `picked_up` is false only when its last run is over and Linear has not
   * changed the issue since: the next check leaves it alone, and only running it again starts it.
   */
  | { kind: "eligible"; picked_up: boolean }
  | { kind: "not_todo"; state: string }
  | { kind: "not_candidate" }
  | { kind: "waiting"; reason: WaitReason }
  | { kind: "paused" };
export interface IssueView { issue: Issue; job: Job | null; status: IssueStatus }

export interface TriggerNames { label: string; todo: string; in_progress: string; in_testing: string; done: string }
export interface NotificationSettings { run_started: boolean; awaiting_acceptance: boolean; needs_info: boolean; failed: boolean }
export interface Settings {
  language: string;
  poll_interval_secs: number;
  max_concurrent: number;
  run_timeout_mins: number;
  stall_timeout_mins: number;
  log_retention_days: number;
  log_max_total_mb: number;
  default_executor: ExecutorKind;
  default_permission: PermissionProfile;
  /** The reasoning effort of bindings that set none; absent, each executor uses its own setting. */
  default_effort?: Effort;
  prompt_template: string;
  /** "follow-ui" or a language code. */
  comment_language: string;
  paused: boolean;
  trigger: TriggerNames;
  notifications: NotificationSettings;
  auto_update: boolean;
  onboarding_completed: boolean;
  full_access_acknowledged: boolean;
  /** Keep the app's icon in the macOS Dock while it runs; off, it shows only while the panel is open. Windows has no Dock. */
  show_in_dock: boolean;
}

export interface Binding {
  id: string;
  project_id: string;
  project_name: string;
  directory: string;
  executor?: ExecutorKind;
  permission?: PermissionProfile;
  model?: string;
  prompt_override?: string;
  /** Whether runs are lean; absent means they are (only `false` is saved). */
  lean?: boolean;
  /** Absent: the default of the settings. */
  effort?: Effort;
}
export interface Project { id: string; name: string; team_ids: string[] }

export type EnvSource = "host" | "login-shell" | "system-settings" | "none";
export interface ExecEnv {
  path: string | null;
  path_source: EnvSource;
  proxy: [string, string][];
  proxy_source: EnvSource;
  problems: string[];
}
export interface ExecutorReport {
  kind: ExecutorKind;
  program: string | null;
  version: string | null;
  logged_in: boolean;
  linear_mcp: boolean;
  problems: string[];
  /** The Linear server lean runs load is there under its exact name and address, and connected. */
  lean_linear_mcp: boolean;
  /** A lean run would be signed in, the same way as a usual one. */
  lean_logged_in: boolean;
  /** Why lean runs cannot use the executor, besides `problems`. */
  lean_problems: string[];
}
export interface WorkspaceReport { label_ok: boolean; missing_states: string[]; teams_checked: string[] }
export interface BindingReport {
  binding_id: string;
  project_name: string;
  directory_exists: boolean;
  executor: ExecutorKind;
  /** Ready for the binding's runs: lean ones when `lean` is `on`, else usual ones. */
  executor_ready: boolean;
  lean: LeanStatus;
  /**
   * The effort its runs start with: its own, else the settings' default; `null` when neither sets
   * one.
   */
  effort: Effort | null;
}
export interface DoctorReport {
  viewer: Viewer | null;
  linear_error: string | null;
  executors: ExecutorReport[];
  workspace: WorkspaceReport | null;
  bindings: BindingReport[];
  env: ExecEnv | null;
}

export type SetupAction =
  | { kind: "add_linear_mcp"; executor: ExecutorKind }
  | { kind: "login_linear_mcp"; executor: ExecutorKind }
  | { kind: "register_control_server"; executor: ExecutorKind }
  | { kind: "remove_control_server"; executor: ExecutorKind };
/** One executor's control-server registration; `unknown` when the check itself failed. */
export type ControlServer = "registered" | "not_registered" | "not_installed" | "unknown";
export interface ControlServers { claude: ControlServer; codex: ControlServer }
/** Why the command line cannot be put on PATH from where the app runs. */
export type CliUnavailable = "missing_cli" | "not_installed" | "development";
/** What the settings show for "Command line on PATH" (core's `CliPathStatus`). */
export type CliPathStatus =
  | { kind: "off" }
  | { kind: "on"; shell_line: string | null }
  | { kind: "stale"; target: string }
  | { kind: "blocked"; path: string }
  | { kind: "unavailable"; reason: CliUnavailable };
export interface AppInfo { version: string; cli_path: string | null; platform: string; system_locale: string | null }
export type StartupCode = "already_running" | "store_unavailable" | "other";
export type StartupState =
  | { kind: "starting" }
  | { kind: "ready" }
  | { kind: "failed"; code: StartupCode; detail: string };
export type EngineEvent = { kind: "status" } | { kind: "job"; job: Job } | { kind: "config" };
export type QuitMode = "wait" | "stop";
/** What follows once the runs are settled: IssueRelay quits, or installs the downloaded update and restarts. */
export type QuitPurpose = "quit" | "update";
/**
 * The question quitting asks (`quit-requested`): the runs in progress, whether quitting already waits
 * for them, and what follows (`QuitPrompt` in quit.rs).
 */
export interface QuitPrompt { running: number; waiting: boolean; purpose: QuitPurpose }

/** The update failure codes (`FailureCode` in update.rs); update-failure-cases.json pins both sides. */
export const UPDATE_FAILURE_CODES = ["unreachable", "no_platform", "bad_signature", "unavailable", "install_failed", "other"] as const;
export type UpdateFailureCode = (typeof UPDATE_FAILURE_CODES)[number];
/** What the app knows about updates (`update-status`, `update_status`, `check_update`; `UpdateStatus` in update.rs). */
export type UpdateStatus =
  | { kind: "unchecked" }
  | { kind: "up_to_date"; checked_at: string }
  | { kind: "available"; version: string }
  | { kind: "failed"; code: UpdateFailureCode; detail: string };

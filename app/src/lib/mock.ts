import { type Backend, BackendError } from "./backend";
import { MOCK_NOW, now, setFixedClock } from "./clock";
import {
  DEFAULT_SETTINGS, DEMO_APP_INFO, DEMO_BINDINGS, DEMO_DOCTOR, DEMO_HELD, DEMO_ISSUES, DEMO_JOBS, DEMO_LOGS,
  DEMO_PROJECTS, DEMO_VIEWER, FRESH_DOCTOR, ago,
} from "./mock-fixtures";
import { isActive } from "./runs";
import type {
  Binding, ControlServers, DoctorReport, EngineEvent, HeldJob, Issue, IssueView, Job, LinearSetup, Settings,
  StartupState, StatusView, Viewer,
} from "./types";

export interface MockOptions {
  /** The delay of every call: 120 ms by default so loading states show in a browser; tests pass 0. */
  latencyMs?: number;
}

const NOT_CONNECTED = "not connected to Linear: log in first";
const CONNECTING = "still connecting to Linear";

/** `https` on linear.app or one of its subdomains: the rule of `is_linear_url` in commands.rs. */
function isLinearUrl(text: string): boolean {
  try {
    const url = new URL(text);
    return url.protocol === "https:" && (url.hostname === "linear.app" || url.hostname.endsWith(".linear.app"));
  } catch {
    return false;
  }
}

/**
 * Whether the next check picks up an issue that meets every condition: the engine's rule
 * (`Engine::issues`, which asks `trigger::should_enqueue`). It has no run, or its newest run is
 * still in hand, or Linear changed it after the moment that run recorded. Otherwise only running
 * it again starts it.
 */
export function pickedUp(issue: Issue, newest: Job | null): boolean {
  return (
    newest === null ||
    isActive(newest) ||
    Date.parse(issue.updated_at) > Date.parse(newest.verified_updated_at ?? newest.discovered_updated_at)
  );
}

/**
 * The panel's backend without Tauri: fictional data in memory that behaves like the engine as far
 * as the panel can see — the same rules and messages for marking done, running again, stopping,
 * saving settings and bindings, and the same events. Development, component tests and screenshots
 * use it; it never touches real data.
 */
export function createMockBackend(scenario: "demo" | "fresh" | "connecting", options: MockOptions = {}): Backend {
  const latencyMs = options.latencyMs ?? 120;
  // Relative times and run durations in tests and screenshots are computed from this moment.
  setFixedClock(MOCK_NOW);
  // "connecting" is the demo install starting up: its local data, Linear not connected yet.
  const demo = scenario !== "fresh";
  let jobs: Job[] = demo ? structuredClone(DEMO_JOBS) : [];
  let held: HeldJob[] = demo ? structuredClone(DEMO_HELD) : [];
  let bindings: Binding[] = demo ? structuredClone(DEMO_BINDINGS) : [];
  let settings: Settings = { ...DEFAULT_SETTINGS, onboarding_completed: demo, full_access_acknowledged: demo };
  let linear: LinearSetup =
    scenario === "demo" ? { kind: "connected", method: "oauth" } : scenario === "connecting" ? { kind: "connecting" } : { kind: "not_configured" };
  let viewer: Viewer | null = scenario === "demo" ? DEMO_VIEWER : null;
  let doctor: DoctorReport = structuredClone(
    scenario === "demo" ? DEMO_DOCTOR : scenario === "connecting" ? { ...DEMO_DOCTOR, viewer: null, linear_error: CONNECTING, workspace: null } : FRESH_DOCTOR,
  );
  const controlServers: ControlServers = { claude: "not_registered", codex: "not_registered" };
  let autostart = false;
  const listeners = {
    event: new Set<(event: EngineEvent) => void>(),
    resync: new Set<() => void>(),
    quit: new Set<(running: number) => void>(),
    startup: new Set<(state: StartupState) => void>(),
  };

  const wait = () => new Promise<void>((resolve) => setTimeout(resolve, latencyMs));
  /** Answers after the delay, reading the state only then. */
  const reply = async <T>(value: () => T): Promise<T> => {
    await wait();
    return value();
  };
  const emit = (event: EngineEvent) => listeners.event.forEach((listener) => listener(event));
  const fail = (message: string): never => {
    throw new BackendError(message);
  };
  const find = (id: number): Job => jobs.find((job) => job.id === id) ?? fail(`job ${id} not found`);
  const connected = () => linear.kind === "connected";
  const connecting = () => linear.kind === "connecting";
  /** What a call that needs Linear fails with while it is not connected. */
  const offline = () => fail(connecting() ? CONNECTING : NOT_CONNECTED);
  const update = (id: number, change: Partial<Job>) => {
    jobs = jobs.map((job) => (job.id === id ? { ...job, ...change } : job));
    emit({ kind: "job", job: find(id) });
    emit({ kind: "status" });
  };
  const subscribe = async <T>(set: Set<T>, listener: T): Promise<() => void> => {
    set.add(listener);
    return () => {
      set.delete(listener);
    };
  };
  const status = (): StatusView => ({
    status: {
      paused: settings.paused,
      linear_ok: connected(),
      viewer,
      // While connecting no check has finished yet.
      last_poll_at: connecting() ? null : ago(0.5),
      last_error: connected() || connecting() ? null : NOT_CONNECTED,
      running: jobs.filter((job) => job.state === "running").length,
      queued: jobs.filter((job) => job.state === "queued").length,
      waiting: jobs.filter((job) => job.state === "waiting_binding").length,
      draining: false,
      held: held.filter((pause) => jobs.some((job) => job.id === pause.job_id && job.state === "queued")),
    },
    linear,
  });
  const newestRun = (issueUuid: string): Job | undefined =>
    jobs.filter((job) => job.issue_uuid === issueUuid).sort((a, b) => b.id - a.id)[0];
  const issueViews = (): IssueView[] =>
    (demo ? DEMO_ISSUES : []).map(({ issue, status }) => {
      const job = newestRun(issue.uuid) ?? null;
      // The verdict of an eligible issue follows its newest run, which stopping or running again changes.
      return { issue, job, status: status.kind === "eligible" ? { kind: "eligible", picked_up: pickedUp(issue, job) } : status };
    });
  const connect = (method: "oauth" | "api_key"): Viewer => {
    linear = { kind: "connected", method };
    viewer = DEMO_VIEWER;
    doctor = { ...doctor, viewer: DEMO_VIEWER, linear_error: null, workspace: DEMO_DOCTOR.workspace };
    emit({ kind: "status" });
    return DEMO_VIEWER;
  };

  return {
    appInfo: () => reply(() => DEMO_APP_INFO),
    startupState: () => reply((): StartupState => ({ kind: "ready" })),
    status: () => reply(status),
    listJobs: (limit) => reply(() => [...jobs].sort((a, b) => b.id - a.id).slice(0, Math.min(Math.max(limit, 1), 500))),
    job: (id) => reply(() => jobs.find((job) => job.id === id) ?? null),
    jobLog: (id, lines) =>
      reply(() => {
        find(id);
        return (DEMO_LOGS[id] ?? []).slice(-lines);
      }),
    issues: () => reply(() => (connected() ? issueViews() : offline())),
    pause: () =>
      reply(() => {
        settings = { ...settings, paused: true };
        emit({ kind: "status" });
      }),
    resume: () =>
      reply(() => {
        settings = { ...settings, paused: false };
        emit({ kind: "status" });
      }),
    pollNow: () => reply(() => emit({ kind: "status" })),
    cancelJob: (id) =>
      reply(() => {
        const job = find(id);
        if (!["queued", "waiting_binding", "running"].includes(job.state)) {
          fail(`job ${id} is ${job.state} and cannot be stopped`);
        }
        // A running job's process tree ends and the run is recorded at once here.
        update(id, { state: "canceled", reason: "stopped by the user", finished_at: now().toISOString() });
      }),
    retryJob: (id) =>
      reply(() => {
        const job = find(id);
        if (!["failed", "canceled", "needs_info"].includes(job.state)) {
          fail(`job ${id} is ${job.state}; only failed, stopped or needs-info runs can be retried`);
        }
        if (newestRun(job.issue_uuid)?.id !== id) {
          fail(`job ${id} is not the latest run of ${job.issue_identifier}; retry that one instead`);
        }
        held = held.filter((pause) => pause.job_id !== id);
        update(id, { state: "queued", reason: null, started_at: null, finished_at: null });
      }),
    markDone: (id) =>
      reply(() => {
        if (find(id).state !== "awaiting_acceptance") fail(`job ${id} is not awaiting acceptance`);
        update(id, { state: "done" });
      }),
    bindings: () => reply(() => structuredClone(bindings)),
    saveBinding: (binding) =>
      reply(() => {
        const others = bindings.filter((b) => b.project_id !== binding.project_id && b.id !== binding.id);
        // The engine refuses a folder that is not absolute; the demo's fictional folders start with ~/.
        if (!binding.directory.startsWith("/") && !binding.directory.startsWith("~/")) {
          fail(`bindings[${others.length}].directory must be an absolute path: ${JSON.stringify(binding.directory)}`);
        }
        bindings = [...others, binding];
        emit({ kind: "config" });
      }),
    removeBinding: (id) =>
      reply(() => {
        const before = bindings.length;
        bindings = bindings.filter((b) => b.id !== id);
        emit({ kind: "config" });
        return bindings.length !== before;
      }),
    projects: () => reply(() => (connected() ? DEMO_PROJECTS : offline())),
    settings: () => reply(() => structuredClone(settings)),
    saveSettings: (next) =>
      reply(() => {
        // Monitoring is switched with pause and resume, never by saving the settings page.
        settings = { ...next, paused: settings.paused };
        emit({ kind: "config" });
      }),
    doctor: () => reply(() => structuredClone(doctor)),
    linearLogin: () => reply(() => connect("oauth")),
    // The fake's logins answer at once, so nothing is ever waiting to be cancelled; tests that
    // need a waiting login replace `linearLogin` and `linearCancelLogin`.
    linearCancelLogin: () => reply(() => undefined),
    linearUseApiKey: (key) => reply(() => (key === "lin_api_bad" ? fail("Linear did not accept the API key") : connect("api_key"))),
    linearLogout: () =>
      reply(() => {
        linear = { kind: "not_configured" };
        viewer = null;
        emit({ kind: "status" });
      }),
    // A start still connecting gets its connection here; otherwise the setup stays as it is.
    linearReconnect: () =>
      reply(() => {
        if (connecting()) connect("oauth");
        return linear;
      }),
    setupAction: (action) =>
      reply(() => {
        doctor = {
          ...doctor,
          executors: doctor.executors.map((report) =>
            report.kind === action.executor && (action.kind === "add_linear_mcp" || action.kind === "login_linear_mcp")
              ? { ...report, linear_mcp: true, problems: report.problems.filter((p) => !p.startsWith("Linear MCP")) }
              : report,
          ),
        };
        if (action.kind === "register_control_server") controlServers[action.executor] = "registered";
        if (action.kind === "remove_control_server") controlServers[action.executor] = "not_registered";
      }),
    controlServers: () => reply(() => ({ ...controlServers })),
    // The demo registered Linear under the names the one-click login uses.
    linearServerNamed: () => reply(() => true),
    openLinear: (url) =>
      reply(() => {
        if (!isLinearUrl(url)) fail("not_a_linear_link");
      }),
    chooseDirectory: () => reply(() => "~/code/new-project"),
    autostartEnabled: () => reply(() => autostart),
    setAutostart: (enabled) =>
      reply(() => {
        autostart = enabled;
      }),
    quit: () => reply(() => undefined),
    requestQuit: () => reply(() => undefined),
    onEngineEvent: (listener) => subscribe(listeners.event, listener),
    onResync: (listener) => subscribe(listeners.resync, listener),
    onQuitRequested: (listener) => subscribe(listeners.quit, listener),
    onStartupFailed: (listener) => subscribe(listeners.startup, listener),
  };
}

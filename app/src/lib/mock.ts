import { type Backend, BackendError } from "./backend";
import { MOCK_NOW, now, setFixedClock } from "./clock";
import {
  DEFAULT_SETTINGS, DEMO_APP_INFO, DEMO_BINDINGS, DEMO_DOCTOR, DEMO_HELD, DEMO_ISSUES, DEMO_JOBS, DEMO_LABELS, DEMO_LOGS,
  DEMO_PROJECTS, DEMO_SHELL_LINE, DEMO_UPDATES, DEMO_VIEWER, FRESH_DOCTOR, ago,
} from "./mock-fixtures";
import type { ModelCatalog } from "./model-catalog";
import { bindingsOverlap } from "./routing";
import { canRecordLocalCompletion, isActive, isInterruptedRun, isPlanned, pendingContinuationSourceId } from "./runs";
import type {
  Binding, CliPathStatus, ControlServers, DoctorReport, EngineEvent, ExecutorKind, HeldJob, Issue, IssueView, Job, LinearSetup,
  QuitPrompt, Settings, StartupState, StatusView, UpdateStatus, Viewer,
} from "./types";
import type { ContinuationPreview, DispatchPreview, Handoff, IssueContext } from "./workflow-types";

export interface MockOptions {
  /** The delay of every call: 120 ms by default so loading states show in a browser; tests pass 0. */
  latencyMs?: number;
  /** The `language` of the settings ("system" or a language code); the screenshots ask for one in the address. */
  language?: string;
  /**
   * What the app's update check has found at the start: `available` offers a new version,
   * `install-failed` tells that an update could not be installed (`DEMO_UPDATES`); any other name,
   * or none, is up to date. The layout checks ask for one in the address.
   */
  update?: string;
  /** Shows a waiting dispatch in the demo for exercising the preview in a browser. */
  dispatchReview?: boolean;
}

const NOT_CONNECTED = "not connected to Linear: log in first";
const CONNECTING = "still connecting to Linear";

/** The model choices shown by the browser preview, without querying an installed CLI. */
function mockCatalog(executor: ExecutorKind): ModelCatalog {
  const entries = executor === "codex"
    ? [["gpt-6.1-sol", "GPT-6.1 Sol"], ["gpt-6-astra", "GPT-6 Astra"], ["gpt-6-luna", "GPT-6 Luna"]]
    : [["fable", "Claude Fable"], ["opus", "Claude Opus"], ["sonnet", "Claude Sonnet"]];
  return {
    executor,
    source: "builtin",
    models: entries.map(([id, label], index) => ({
      id,
      label,
      efforts: ["low", "medium", "high", "xhigh", "max", ...(executor === "codex" && index < 2 ? ["ultra"] : [])],
      default_effort: null,
      recommended: executor === "claude" || index < 2,
      is_default: index === 0,
    })),
  };
}

/**
 * `https` on linear.app or one of its subdomains: close to `linear_url` in commands.rs, for the
 * browser preview only (the app's own check is that one; SECURITY_CODING_SPEC.md section 5).
 */
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
  const claims = new Map<number, { id: string; executor: ExecutorKind }>();
  let nextClaimId = 1;
  let dispatchReviewed = false;
  let dispatchToken: string | null = null;
  let nextDispatchToken = 1;
  const continuationPreviews = new Map<number, ContinuationPreview>();
  const continuationContexts = new Map<string, IssueContext>();
  let nextContinuationToken = 1;
  let held: HeldJob[] = demo ? structuredClone(DEMO_HELD) : [];
  let bindings: Binding[] = demo ? structuredClone(DEMO_BINDINGS) : [];
  let settings: Settings = {
    ...DEFAULT_SETTINGS,
    language: options.language ?? DEFAULT_SETTINGS.language,
    onboarding_completed: demo,
    full_access_acknowledged: demo,
  };
  let linear: LinearSetup =
    scenario === "demo" ? { kind: "connected", method: "oauth" } : scenario === "connecting" ? { kind: "connecting" } : { kind: "not_configured" };
  let viewer: Viewer | null = scenario === "demo" ? DEMO_VIEWER : null;
  let doctor: DoctorReport = structuredClone(
    scenario === "demo" ? DEMO_DOCTOR : scenario === "connecting" ? { ...DEMO_DOCTOR, viewer: null, linear_error: CONNECTING, workspace: null } : FRESH_DOCTOR,
  );
  const controlServers: ControlServers = { claude: "not_registered", codex: "not_registered" };
  let autostart = false;
  let cliPath: CliPathStatus = { kind: "off" };
  // What the app's last update check found; the fake never sends `update-status` by itself.
  const updateState: UpdateStatus =
    (options.update !== undefined && Object.hasOwn(DEMO_UPDATES, options.update) ? DEMO_UPDATES[options.update] : undefined) ?? {
      kind: "up_to_date",
      checked_at: ago(30),
    };
  const listeners = {
    event: new Set<(event: EngineEvent) => void>(),
    resync: new Set<() => void>(),
    quit: new Set<(prompt: QuitPrompt) => void>(),
    startup: new Set<(state: StartupState) => void>(),
    update: new Set<(status: UpdateStatus) => void>(),
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
  const continuation = (id: number) => {
    const source = find(id);
    const latest = newestRun(source.issue_uuid);
    if (!isInterruptedRun(source) || source.directory === null || latest === undefined ||
        (latest.id !== id && pendingContinuationSourceId(latest) !== id)) {
      return fail("only the latest interrupted run can be continued");
    }
    return { source, latest, directory: source.directory };
  };
  const handoff = (job: Job): Handoff => ({
    job_id: job.id,
    issue_identifier: job.issue_identifier,
    executor: job.executor,
    active: isActive(job),
    context: null,
    memory: null,
    prompt: `Continue the work for ${job.issue_identifier} from run ${job.id}.`,
    review_prompt: `Independently review the work for ${job.issue_identifier} from run ${job.id}.`,
    claimed_by: claims.get(job.id)?.executor ?? null,
    claim_id: claims.get(job.id)?.id ?? null,
    archive: settings.wiki_root === undefined ? null : {
      root: settings.wiki_root,
      prompt: `Archive the work for ${job.issue_identifier} from run ${job.id} in the configured Wiki.`,
      receipt: null,
    },
  });
  const issueViews = (): IssueView[] =>
    (demo ? DEMO_ISSUES : []).map(({ issue, status }) => {
      const job = newestRun(issue.uuid) ?? null;
      if (options.dispatchReview && issue.identifier === "DEMO-3" && !dispatchReviewed) {
        return { issue, job, status: { kind: "waiting", reason: { code: "dispatch_review_required" } } };
      }
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
    readCodexSession: (id) => reply(() => {
      const job = find(id);
      if (job.executor !== "codex" || job.finished_at === null || job.session_id == null) {
        return fail("select a finished Codex run with a recorded session");
      }
      return {
        session_id: job.session_id,
        title: job.title,
        messages: [
          { role: "user", text: "Check the implementation and run its tests." },
          { role: "assistant", text: "The focused checks passed." },
        ],
        has_more: false,
      };
    }),
    getHandoff: (id) => reply(() => handoff(find(id))),
    claimHandoff: (id, executor) => reply(() => {
      const job = find(id);
      if (isActive(job) || job.state === "done") fail("run is active or already done");
      if (newestRun(job.issue_uuid)?.id !== id) fail("only the latest run can be claimed");
      if (job.directory === null) fail("run has no working directory");
      const sameWork = (other: Job) => other.issue_uuid === job.issue_uuid || other.directory === job.directory;
      if (jobs.some((other) => other.state === "running" && sameWork(other))) fail("another writer is still running in the directory");
      if (jobs.some((other) => claims.has(other.id) && sameWork(other))) fail("the run, issue or directory is already claimed");
      claims.set(id, { id: `demo-claim-${nextClaimId++}`, executor });
      emit({ kind: "job", job });
      return handoff(job);
    }),
    releaseHandoff: (id, claimId) => reply(() => {
      const claim = claims.get(id);
      if (claim === undefined || claim.id !== claimId) return false;
      claims.delete(id);
      emit({ kind: "job", job: find(id) });
      return true;
    }),
    issues: () => reply(() => (connected() ? issueViews() : offline())),
    getDispatchPreview: (issueIdentifier) => reply((): DispatchPreview => {
      if (!connected()) return offline();
      const candidate = issueViews().find(({ issue, status }) =>
        issue.identifier === issueIdentifier && status.kind === "waiting" && status.reason.code === "dispatch_review_required");
      if (candidate === undefined) return fail("this issue is not waiting for dispatch review");
      dispatchToken = `demo-dispatch-${nextDispatchToken++}`;
      return {
        token: dispatchToken,
        issue_uuid: candidate.issue.uuid,
        issue_identifier: candidate.issue.identifier,
        title: candidate.issue.title,
        mode: "implement",
        executor: "claude",
        directory: "~/code/demo-app",
        model: null,
        effort: null,
        wiki_root: settings.wiki_root ?? null,
        context: {
          revision: 0,
          origin_executor: null,
          preferred_executor: null,
          request: "The old CI badge points to the retired build. Update it after checking the current workflow name.",
          conversation_history: "The team confirmed the former build was retired. The current workflow name is in the repository.",
          scope: "Update the README badge for the demo project.",
          acceptance: ["The badge links to the current CI workflow."],
          source_refs: ["wiki/ci.md#status-badge"],
        },
      };
    }),
    approveDispatch: (issueIdentifier, token) => reply(() => {
      if (!connected()) return offline();
      if (issueIdentifier !== "DEMO-3" || token !== dispatchToken || dispatchReviewed) {
        return fail("the dispatch preview changed; read it again");
      }
      dispatchToken = null;
      dispatchReviewed = true;
    }),
    getContinuationContext: (id) => reply(() => {
      const source = find(id);
      return continuationContexts.get(source.issue_uuid) ?? handoff(source).context;
    }),
    supplementContinuationContext: (id, expectedRevision, supplement) => reply(() => {
      const source = find(id);
      const current = continuationContexts.get(source.issue_uuid) ?? handoff(source).context;
      if ((current?.revision ?? 0) !== expectedRevision) return fail("issue context revision changed; read it and try again");
      if (!supplement.trim()) return fail("provide additional task information");
      const updated: IssueContext = {
        ...(current ?? { origin_executor: null, preferred_executor: null, request: source.title, scope: "", acceptance: [], source_refs: [] }),
        revision: expectedRevision + 1,
        conversation_history: [current?.conversation_history, supplement].filter(Boolean).join("\n\n"),
      };
      continuationContexts.set(source.issue_uuid, updated);
      continuationPreviews.delete(id);
      return structuredClone(updated);
    }),
    getContinuationPreview: (id, executor) => reply(() => {
      if (!connected()) return offline();
      const { source, directory } = continuation(id);
      const canResume = executor === source.executor && source.session_id != null && settings.resume_sessions;
      const preview: ContinuationPreview = {
        token: `demo-continuation-${nextContinuationToken++}`,
        source_job_id: id,
        source_executor: source.executor,
        issue_uuid: source.issue_uuid,
        issue_identifier: source.issue_identifier,
        title: source.title,
        mode: source.mode ?? "implement",
        executor,
        strategy: canResume ? "resume_session" : "handoff",
        directory,
        model: (executor === "codex" ? settings.default_codex_model : settings.default_claude_model) || null,
        effort: settings.default_effort ?? null,
        permission: settings.default_permission,
        claude_ultracode: executor === "claude" && source.mode !== "plan" && settings.claude_ultracode === true,
        context: continuationContexts.get(source.issue_uuid) ?? handoff(source).context,
        wiki_root: settings.wiki_root ?? null,
        evidence: [source.reason, source.summary].filter(Boolean).join("\n"),
      };
      continuationPreviews.set(id, preview);
      return preview;
    }),
    approveContinuation: (id, token) => reply(() => {
      if (!connected()) return offline();
      const { source, latest } = continuation(id);
      const preview = continuationPreviews.get(id);
      if (preview === undefined || preview.token !== token) return fail("the continuation preview changed; read it again");
      continuationPreviews.delete(id);
      const jobId = latest.id === id ? Math.max(...jobs.map((job) => job.id)) + 1 : latest.id;
      const job: Job = {
        ...latest,
        id: jobId,
        created_at: latest.id === id ? now().toISOString() : latest.created_at,
        executor: preview.executor,
        state: "queued",
        reason: null,
        started_at: null,
        finished_at: null,
        log_path: null,
        summary: null,
        report: null,
        session_id: null,
        resume_job_id: source.id,
        usage: null,
      };
      jobs = [job, ...jobs.filter((row) => row.id !== jobId)];
      emit({ kind: "job", job });
      emit({ kind: "status" });
      return jobId;
    }),
    getDevelopmentIssueUrl: (id) => reply(() => {
      if (!connected()) return offline();
      const job = find(id);
      if (!isPlanned(job) || job.finished_at === null) return fail("this run has not posted a plan");
      return `https://linear.app/demo/issue/${job.issue_identifier}`;
    }),
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
        // As `Engine::cancel_job`: only a run still in hand can be stopped.
        if (!isActive(job)) fail(`job ${id} is ${job.state} and cannot be stopped`);
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
    recordLocalCompletion: (id) =>
      reply(() => {
        const job = find(id);
        if (!canRecordLocalCompletion(job)) fail("only a finished implementation waiting for information can be recorded as complete locally");
        if (newestRun(job.issue_uuid)?.id !== id || claims.has(id)) fail("this run is superseded or claimed");
        update(id, { state: "local_complete" });
      }),
    bindings: () => reply(() => structuredClone(bindings)),
    saveBinding: (binding) =>
      reply(() => {
        // As `Engine::add_binding` does: it replaces the binding with its id and every binding that
        // would take the same issues, and takes the place of the first of them (so that editing one
        // does not move it); a binding that replaces none goes last.
        const replaced = (b: Binding) => b.id === binding.id || bindingsOverlap(b, binding);
        const place = bindings.findIndex(replaced);
        const next = bindings.filter((b) => !replaced(b));
        next.splice(place === -1 ? next.length : place, 0, binding);
        // The engine refuses a folder that is not absolute (the demo's fictional folders start with
        // ~/) and names the binding by its place in the list.
        if (!binding.directory.startsWith("/") && !binding.directory.startsWith("~/")) {
          fail(`bindings[${next.indexOf(binding)}].directory must be an absolute path: ${JSON.stringify(binding.directory)}`);
        }
        bindings = next;
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
    labels: () => reply(() => (connected() ? [...DEMO_LABELS] : offline())),
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
            // The setup commands use the names lean runs load, so lean runs get Linear too.
            report.kind === action.executor && (action.kind === "add_linear_mcp" || action.kind === "login_linear_mcp")
              ? {
                  ...report,
                  linear_mcp: true,
                  problems: report.problems.filter((p) => !p.startsWith("Linear MCP")),
                  lean_linear_mcp: true,
                  lean_problems: report.lean_problems.filter((p) => !p.includes("Linear MCP server")),
                }
              : report,
          ),
        };
        if (action.kind === "register_control_server") controlServers[action.executor] = "registered";
        if (action.kind === "remove_control_server") controlServers[action.executor] = "not_registered";
      }),
    controlServers: () => reply(() => ({ ...controlServers })),
    modelCatalog: (executor) => reply(() => mockCatalog(executor)),
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
    cliOnPath: () => reply(() => cliPath),
    installCliOnPath: () =>
      reply(() => {
        // The demo user's login shell does not have ~/.local/bin yet, so the line is shown.
        cliPath = { kind: "on", shell_line: DEMO_SHELL_LINE };
        return cliPath;
      }),
    removeCliFromPath: () =>
      reply(() => {
        cliPath = { kind: "off" };
        return cliPath;
      }),
    quit: () => reply(() => undefined),
    requestQuit: () => reply(() => undefined),
    updateStatus: () => reply(() => updateState),
    checkUpdate: () => reply(() => updateState),
    installUpdate: () => reply(() => undefined),
    onEngineEvent: (listener) => subscribe(listeners.event, listener),
    onResync: (listener) => subscribe(listeners.resync, listener),
    onQuitRequested: (listener) => subscribe(listeners.quit, listener),
    onStartupFailed: (listener) => subscribe(listeners.startup, listener),
    onUpdateStatus: (listener) => subscribe(listeners.update, listener),
  };
}

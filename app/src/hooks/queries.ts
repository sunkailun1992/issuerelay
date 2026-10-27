import { type QueryKey, useIsMutating, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { type Backend, BackendError } from "@/lib/backend";
import { useBackend } from "@/lib/backend-context";
import { reportFailure } from "@/lib/errors";
import type { Binding, CliPathStatus, ExecutorKind, LinearSetup, Settings, SetupAction, UpdateStatus, Viewer } from "@/lib/types";

/**
 * Every query of the panel. The engine events (`useEngineEvents`) and the actions below refresh
 * data by these keys, and a key that is the start of another refreshes it too (`["jobs"]`
 * refreshes `["jobs", 200]`), so a query is keyed by its name first and its parameters after.
 */
export const queryKeys = {
  startup: ["startup"] as const,
  status: ["status"] as const,
  jobs: ["jobs"] as const,
  job: (id: number) => ["job", id] as const,
  log: (id: number) => ["log", id] as const,
  issues: ["issues"] as const,
  bindings: ["bindings"] as const,
  projects: ["projects"] as const,
  labels: ["labels"] as const,
  settings: ["settings"] as const,
  doctor: ["doctor"] as const,
  controlServers: ["controlServers"] as const,
  linearServerNamed: (executor: ExecutorKind) => ["linearServerNamed", executor] as const,
  autostart: ["autostart"] as const,
  cliPath: ["cliPath"] as const,
  update: ["update"] as const,
  appInfo: ["appInfo"] as const,
};

/**
 * How many lines of a log are read (`MAX_LOG_LINES` in the app's commands): its tail. A log that
 * comes back this long may have begun earlier, which the run's page says.
 */
export const LOG_LINES = 5000;

/** How many runs the list of runs reads, newest first; a list this long may leave older ones out. */
export const RUN_LIST_LIMIT = 200;

/** How often a running job's log is read again. */
const LOG_REFRESH_MS = 1500;

/** How often the start is asked about again while it is under way. */
const STARTUP_REFRESH_MS = 1000;

/**
 * Whether the host has started. Asked again every second while it is starting, and asked at all
 * (rather than waiting for `startup-failed`) so that a panel opened after a failure sees it.
 */
export function useStartupState() {
  const backend = useBackend();
  return useQuery({
    queryKey: queryKeys.startup,
    queryFn: () => backend.startupState(),
    refetchInterval: (query) => (query.state.data?.kind === "starting" ? STARTUP_REFRESH_MS : false),
  });
}

/** The monitoring status and the Linear connection (`StatusView`). */
export function useStatus() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.status, queryFn: () => backend.status() });
}

/** The most recent runs, newest first. */
export function useJobs(limit = RUN_LIST_LIMIT) {
  const backend = useBackend();
  return useQuery({ queryKey: [...queryKeys.jobs, limit], queryFn: () => backend.listJobs(limit) });
}

/** One run, or `null` when there is none with this id. */
export function useJob(id: number) {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.job(id), queryFn: () => backend.job(id) });
}

/** The tail of a run's log; read again every 1.5 seconds while the run is `live`. */
export function useJobLog(id: number, { live }: { live: boolean }) {
  const backend = useBackend();
  return useQuery({
    queryKey: queryKeys.log(id),
    queryFn: () => backend.jobLog(id, LOG_LINES),
    refetchInterval: live ? LOG_REFRESH_MS : false,
  });
}

/** The issues with the trigger label and what IssueRelay makes of each. Asks Linear. */
export function useIssues() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.issues, queryFn: () => backend.issues() });
}

export function useBindings() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.bindings, queryFn: () => backend.bindings() });
}

/** The Linear projects that can be bound. Asks Linear, so only when `enabled`. */
export function useProjects(enabled = true) {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.projects, queryFn: () => backend.projects(), enabled });
}

/** Linear's labels, which a binding's routing labels are chosen from. Asks Linear, so only when `enabled`. */
export function useLabels(enabled = true) {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.labels, queryFn: () => backend.labels(), enabled });
}

export function useSettings() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.settings, queryFn: () => backend.settings() });
}

/**
 * How long an environment check counts as current where a page asks for it again soon (the guide,
 * the overview's note): five minutes, the time the engine keeps a confirmed result for (design 6.4).
 * A report that a change has outdated is checked again whatever its age.
 */
export const DOCTOR_FRESH_MS = 5 * 60_000;

/**
 * The environment check. It runs the executors and asks Linear, so only when `enabled`. A page that
 * asks for it again is shown the report it has, instead of a new check (which takes up to a
 * minute), when the report is younger than `staleTime`. A report that a change has outdated (a fix,
 * a binding, the settings, the Linear connection) is checked again whatever its age.
 */
export function useDoctor(enabled = true, { staleTime }: { staleTime?: number } = {}) {
  const backend = useBackend();
  return useQuery({
    queryKey: queryKeys.doctor,
    queryFn: () => backend.doctor(),
    enabled,
    ...(staleTime === undefined ? {} : { staleTime }),
  });
}

/** Whether each executor has IssueRelay's control server registered. */
export function useControlServers() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.controlServers, queryFn: () => backend.controlServers() });
}

/** Whether the executor knows Linear's MCP server under the name the one-click login uses. */
export function useLinearServerNamed(executor: ExecutorKind, enabled = true) {
  const backend = useBackend();
  return useQuery({
    queryKey: queryKeys.linearServerNamed(executor),
    queryFn: () => backend.linearServerNamed(executor),
    enabled,
  });
}

export function useAutostart() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.autostart, queryFn: () => backend.autostartEnabled() });
}

/** Whether the command line is on PATH through IssueRelay, and what is still to do (design 12). */
export function useCliPath() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.cliPath, queryFn: () => backend.cliOnPath() });
}

/** What the app's last update check found; `update-status` keeps it current (`useEngineEvents`). */
export function useUpdateStatus() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.update, queryFn: () => backend.updateStatus() });
}

/** Version, command line path, platform and system language; fixed while the app runs. */
export function useAppInfo() {
  const backend = useBackend();
  return useQuery({ queryKey: queryKeys.appInfo, queryFn: () => backend.appInfo(), staleTime: Infinity });
}

/**
 * An action on the backend. When it succeeds the queries it changed are refreshed (given as keys,
 * or as a function of what the action was called with); when it fails the user is told with the
 * translated headline `failureKey` and the backend's line as the reason (`reportFailure`).
 * `quiet` can keep a failure that is no failure from being reported. `gcTime` is how long the
 * action's record (what it was called with, and how it went) is kept once nothing shows it any more; the
 * library keeps it five minutes unless told otherwise. The keys in `reread` are
 * also waited for: the action is over once those are read again, and read again wherever they
 * are cached, also when no page shows them right now (`refetchType: "all"`), so that whatever
 * the user opens next, the first-run gate included, finds the saved values and not the ones from
 * before. That is for cheap local data; everything else is refreshed in the background, for the
 * pages that show it, since the action is done when the backend says so and not when every page,
 * the environment check included, has read its data again.
 */
function useAction<Variables, Result>(
  run: (backend: Backend, variables: Variables) => Promise<Result>,
  refreshes: readonly QueryKey[] | ((variables: Variables) => readonly QueryKey[]),
  failureKey: string,
  {
    mutationKey,
    quiet,
    reread = [],
    gcTime,
  }: { mutationKey?: QueryKey; quiet?: (error: unknown) => boolean; reread?: readonly QueryKey[]; gcTime?: number } = {},
) {
  const backend = useBackend();
  const client = useQueryClient();
  const { t } = useTranslation();
  return useMutation<Result, Error, Variables>({
    mutationKey,
    ...(gcTime === undefined ? {} : { gcTime }),
    mutationFn: (variables) => run(backend, variables),
    onSuccess: async (_result, variables) => {
      for (const queryKey of typeof refreshes === "function" ? refreshes(variables) : refreshes) {
        void client.invalidateQueries({ queryKey });
      }
      await Promise.all(reread.map((queryKey) => client.invalidateQueries({ queryKey, refetchType: "all" })));
    },
    onError: (error) => {
      if (!quiet?.(error)) reportFailure(error, t, failureKey);
    },
  });
}

/** What a change of a run touches: the counts, the lists, the run's own pages and the issues. */
const jobChange = (id: number): readonly QueryKey[] => [
  queryKeys.status, queryKeys.jobs, queryKeys.job(id), queryKeys.log(id), queryKeys.issues,
];
/** What a change of the Linear connection touches. */
const linearChange: readonly QueryKey[] = [queryKeys.status, queryKeys.issues, queryKeys.projects, queryKeys.labels, queryKeys.doctor];

export function usePause() {
  return useAction<void, void>((backend) => backend.pause(), [queryKeys.status, queryKeys.settings], "actions.failed.pause");
}

export function useResume() {
  return useAction<void, void>((backend) => backend.resume(), [queryKeys.status, queryKeys.settings], "actions.failed.resume");
}

export function usePollNow() {
  return useAction<void, void>(
    (backend) => backend.pollNow(),
    [queryKeys.status, queryKeys.jobs, queryKeys.issues],
    "actions.failed.poll_now",
  );
}

export function useCancelJob() {
  return useAction<number, void>((backend, id) => backend.cancelJob(id), jobChange, "actions.failed.cancel");
}

export function useRetryJob() {
  return useAction<number, void>((backend, id) => backend.retryJob(id), jobChange, "actions.failed.retry");
}

export function useMarkDone() {
  return useAction<number, void>((backend, id) => backend.markDone(id), jobChange, "actions.failed.mark_done");
}

/**
 * What a change of the bindings touches besides the list of bindings itself (which is read again
 * before the action is over): they decide which issues wait and what the environment check looks at.
 */
const bindingChange: readonly QueryKey[] = [queryKeys.issues, queryKeys.doctor, queryKeys.status];

/**
 * `quiet` is for the form that shows a failure itself, where it was asked for (from the mutation's
 * `error`), so that it is not also told as a toast.
 */
export function useSaveBinding({ quiet = false }: { quiet?: boolean } = {}) {
  return useAction<Binding, void>((backend, binding) => backend.saveBinding(binding), bindingChange, "actions.failed.save_binding", {
    reread: [queryKeys.bindings],
    quiet: quiet ? () => true : undefined,
  });
}

/** Resolves to whether there was a binding with this id to remove. */
export function useRemoveBinding() {
  return useAction<string, boolean>((backend, id) => backend.removeBinding(id), bindingChange, "actions.failed.remove_binding", {
    reread: [queryKeys.bindings],
  });
}

/**
 * The saved settings are read again before this is over, so the page that goes on (the guide to the
 * overview) finds them. `quiet` is for the caller that tells a failure itself, after it has put
 * back what it changed for the sake of the save (the guide's language), so that the failure is
 * told in the language the panel is in again.
 */
export function useSaveSettings({ quiet = false }: { quiet?: boolean } = {}) {
  return useAction<Settings, void>(
    (backend, settings) => backend.saveSettings(settings),
    [queryKeys.status, queryKeys.issues, queryKeys.doctor],
    "actions.failed.save_settings",
    { reread: [queryKeys.settings], quiet: quiet ? () => true : undefined },
  );
}

/** The key of the Linear login while it waits for the browser (`useLinearLoginWaiting`). */
const LINEAR_LOGIN = ["linear_login"] as const;

/**
 * The browser login. `mutate()` opens the authorization page and waits for the browser to come
 * back; `cancel()` gives up on it (core drops a login it is not waiting for any more; nothing was
 * stored before the browser answered). A login that ends as cancelled is the user's own doing and
 * is not reported.
 */
export function useLinearLogin() {
  const current = useBackend();
  const { t } = useTranslation();
  const login = useAction<void, Viewer>((backend) => backend.linearLogin(), linearChange, "actions.failed.linear_login", {
    mutationKey: LINEAR_LOGIN,
    quiet: (error) => error instanceof BackendError && error.code === "login_cancelled",
  });
  const cancel = () =>
    current.linearCancelLogin().catch((error: unknown) => reportFailure(error, t, "actions.failed.linear_login"));
  return { ...login, cancel };
}

/**
 * Whether a browser login is waiting. While it is, nothing else may change the Linear login (core
 * does one change at a time), so wherever the API key is offered it must wait too.
 */
export function useLinearLoginWaiting(): boolean {
  return useIsMutating({ mutationKey: LINEAR_LOGIN }) > 0;
}

/**
 * Connects with a personal API key. The key is what the action is called with, so its record is not
 * kept at all once the form has let go of it (`reset()`), not even for the library's five minutes.
 */
export function useLinearApiKey() {
  return useAction<string, Viewer>(
    (backend, key) => backend.linearUseApiKey(key),
    linearChange,
    "actions.failed.linear_api_key",
    { gcTime: 0 },
  );
}

export function useLinearLogout() {
  return useAction<void, void>((backend) => backend.linearLogout(), linearChange, "actions.failed.linear_logout");
}

export function useLinearReconnect() {
  return useAction<void, LinearSetup>(
    (backend) => backend.linearReconnect(),
    linearChange,
    "actions.failed.linear_reconnect",
  );
}

export function useSetupAction() {
  return useAction<SetupAction, void>(
    (backend, action) => backend.setupAction(action),
    // The server name is a prefix: it refreshes the answer for both executors.
    [queryKeys.doctor, queryKeys.controlServers, ["linearServerNamed"]],
    "actions.failed.setup_action",
  );
}

export function useSetAutostart() {
  return useAction<boolean, void>(
    (backend, enabled) => backend.setAutostart(enabled),
    [queryKeys.autostart],
    "actions.failed.set_autostart",
  );
}

export function useInstallCliOnPath() {
  return useAction<void, CliPathStatus>((backend) => backend.installCliOnPath(), [queryKeys.cliPath], "actions.failed.cli_path");
}

export function useRemoveCliFromPath() {
  return useAction<void, CliPathStatus>((backend) => backend.removeCliFromPath(), [queryKeys.cliPath], "actions.failed.cli_path");
}

/** Checks for an update now: what this check found is the action's `data`; the kept status is read again. */
export function useCheckUpdate() {
  return useAction<void, UpdateStatus>((backend) => backend.checkUpdate(), [queryKeys.update], "actions.failed.check_update");
}

/**
 * The settings row for updates: what the app keeps (`useUpdateStatus`), with the result of the
 * row's own check on top for as long as it is the last word. The kept status cannot tell a failed
 * check while it keeps an offer (the banner goes on offering it), so the button's result is shown;
 * but only until the kept status is something else, because then a later check, the background
 * one say, has found something newer. The status read again and found the same is the same
 * object (structural sharing), which is no news.
 */
export function useUpdateCheck() {
  const client = useQueryClient();
  const status = useUpdateStatus();
  const check = useCheckUpdate();
  const [found, setFound] = useState<{ outcome: UpdateStatus; kept: UpdateStatus | undefined } | null>(null);
  const { mutate } = check;
  const run = useCallback(
    () =>
      mutate(undefined, {
        onSuccess: (outcome) => setFound({ outcome, kept: client.getQueryData<UpdateStatus>(queryKeys.update) }),
      }),
    [client, mutate],
  );
  const shown = found !== null && status.data === found.kept ? found.outcome : status.data;
  return { shown, run, isPending: check.isPending };
}

/** "Install and restart": the Rust side downloads, verifies and goes through the quit flow. */
export function useInstallUpdate() {
  return useAction<void, void>((backend) => backend.installUpdate(), [], "actions.failed.install_update");
}

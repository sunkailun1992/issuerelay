import { DISPLAY_JOB_STATES, type DisplayJobState, type ExecutorKind, type Job, type JobState, type PlanProgress } from "./types";
import type { VerificationStatus } from "./workflow-types";

/** The states of a run that is still in hand: it waits, is queued or runs (core's `JobState::is_active`). */
export const ACTIVE_STATES: readonly JobState[] = ["waiting_binding", "queued", "running"];

/** Whether the run is in hand: it can be stopped, and the engine records no new run of its issue meanwhile. */
export function isActive(job: Job): boolean {
  return ACTIVE_STATES.includes(job.state);
}

/** Whether the run was a planning run: it only wrote a plan (run-improvements design, section 10). */
export function isPlanRun(job: Job): boolean {
  return job.mode === "plan";
}

/** Whether the run is a planning run that posted its plan: it waits for the user to decide. */
export function isPlanned(job: Job): boolean {
  return isPlanRun(job) && job.state === "needs_info" && job.report === "planned";
}

/** The progress last read for this planning run, or `null` when there is none. */
export function planProgressFor(progress: readonly PlanProgress[] | undefined, jobId: number): PlanProgress | null {
  return progress?.find((entry) => entry.job_id === jobId) ?? null;
}

/** The later of two readings of one plan's progress (by `checked_at`), or the one there is. */
export function newerProgress(a: PlanProgress | null, b: PlanProgress | null): PlanProgress | null {
  if (a === null) return b;
  if (b === null) return a;
  return Date.parse(b.checked_at) > Date.parse(a.checked_at) ? b : a;
}

/** The persisted reason from core's dispatch-review wait state. */
export function isWaitingForDispatchReview(job: Job): boolean {
  return job.state === "waiting_binding" && job.reason === "waiting for dispatch review";
}

/** An implementation paused after the executor explicitly asked for more information. */
export function isNeedsInfoImplementation(job: Job | null | undefined): boolean {
  return job?.started_at != null && job.state === "needs_info" && job.mode === "implement" && job.report === "needs_info";
}

/** A finished implementation that can be recorded as complete locally from its detail page. */
export function canRecordLocalCompletion(job: Job): boolean {
  return isNeedsInfoImplementation(job) && job.finished_at !== null;
}

/** A started attempt that stopped and can be considered for continuation. */
export function isInterruptedRun(job: Job | null | undefined): job is Job & {
  state: "failed" | "canceled" | "needs_info";
  started_at: string;
  executor: ExecutorKind;
} {
  return job?.started_at != null && job.executor !== null && job.mode !== "verify" &&
    (job.state === "failed" || job.state === "canceled" || isNeedsInfoImplementation(job));
}

/** An unstarted follow-up may be reused to continue the earlier interrupted attempt. */
export function pendingContinuationSourceId(job: Job | null | undefined): number | null {
  if (job?.mode !== "verify" && job?.started_at === null && (job.state === "queued" || job.state === "waiting_binding" || job.state === "canceled")) {
    return job.resume_job_id ?? null;
  }
  return null;
}

/** The newest run (the greatest id) of each issue among `jobs`, whatever their order. */
export function newestRuns(jobs: readonly Job[]): Job[] {
  const newest = new Map<string, Job>();
  for (const job of jobs) {
    const known = newest.get(job.issue_uuid);
    if (known === undefined || job.id > known.id) newest.set(job.issue_uuid, job);
  }
  return [...newest.values()];
}

/**
 * Whether no run in `jobs` is newer than `job` for the same issue. The engine only runs the newest
 * run of an issue again, so the panel offers "Run again" only for such a run.
 */
export function isNewestRun(job: Job, jobs: readonly Job[]): boolean {
  return !jobs.some((other) => other.issue_uuid === job.issue_uuid && other.id > job.id);
}

/** The state shown in the panel. Actions that affect Linear still use the persisted state. */
export function displayJobState(state: JobState): DisplayJobState {
  return state === "local_complete" ? "awaiting_acceptance" : state;
}

/** Verification records keep runtime state apart from the reported acceptance verdict. */
export function displayedVerificationStatus(job: Job): VerificationStatus | undefined {
  if (job.mode !== "verify") return undefined;
  if (job.state === "waiting_binding" || job.state === "queued" || job.state === "running") return job.state;
  if (job.report === "verification_passed") return "passed";
  if (job.report === "verification_failed") return "failed";
  return "blocked";
}

/** The visible state filters. The old local-completion address remains an alias. */
export const RUN_FILTERS = ["all", ...DISPLAY_JOB_STATES] as const;
export type RunFilter = (typeof RUN_FILTERS)[number];

/** The filter an address asks for; anything the page does not know means every run. */
export function parseRunFilter(value: string | null): RunFilter {
  if (value === "local_complete") return displayJobState(value);
  return RUN_FILTERS.find((filter) => filter === value) ?? "all";
}

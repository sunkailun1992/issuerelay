import type { Job, JobState } from "./types";

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

/**
 * What `/runs?state=` accepts: every state a run can be in, or "all". The overview's reminder links
 * to `/runs?state=failed`.
 */
export const RUN_FILTERS = [
  "all",
  "waiting_binding",
  "queued",
  "running",
  "awaiting_acceptance",
  "needs_info",
  "done",
  "failed",
  "canceled",
] as const;
export type RunFilter = (typeof RUN_FILTERS)[number];

/** The filter an address asks for; anything the page does not know means every run. */
export function parseRunFilter(value: string | null): RunFilter {
  return RUN_FILTERS.find((filter) => filter === value) ?? "all";
}

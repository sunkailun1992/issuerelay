import type { Job, JobState } from "./types";

/** The states of a run that is still in hand: it waits, is queued or runs (core's `JobState::is_active`). */
export const ACTIVE_STATES: readonly JobState[] = ["waiting_binding", "queued", "running"];

/** Whether the run is in hand: it can be stopped, and the engine records no new run of its issue meanwhile. */
export function isActive(job: Job): boolean {
  return ACTIVE_STATES.includes(job.state);
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

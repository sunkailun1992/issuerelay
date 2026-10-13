import type { Job } from "./types";

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

import type { Job } from "./types";

export interface IssueRunGroup {
  key: string;
  latest: Job;
  runs: Job[];
}

/** Run ids determine recency. An absent issue UUID leaves each run separate. */
export function groupIssueRuns(jobs: readonly Job[]): IssueRunGroup[] {
  const groups = new Map<string, Job[]>();
  for (const job of jobs) {
    const key = job.issue_uuid.trim() === "" ? `run:${job.id}` : `issue:${job.issue_uuid}`;
    const runs = groups.get(key);
    if (runs === undefined) groups.set(key, [job]);
    else runs.push(job);
  }
  return [...groups].map(([key, runs]) => {
    runs.sort((left, right) => right.id - left.id);
    return { key, latest: runs[0], runs };
  }).sort((left, right) => right.latest.id - left.latest.id);
}

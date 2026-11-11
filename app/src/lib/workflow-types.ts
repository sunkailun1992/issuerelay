import type { ExecutorKind } from "./types";

/** The request and references registered for one issue revision. */
export interface IssueContext {
  revision: number;
  origin_executor: ExecutorKind | null;
  preferred_executor: ExecutorKind | null;
  request: string;
  scope: string;
  acceptance: string[];
  source_refs: string[];
}

/** The saved decisions and facts of a run, separate from the original request and from its log. */
export interface RunMemory {
  decision: { executor: ExecutorKind; source: "explicit" | "binding" | "origin" | "global" } | null;
  checkpoint: {
    summary: string;
    done: string[];
    next: string[];
    verification: string[];
    blockers: string[];
  } | null;
  interruption: { code: string; reset_at: string | null } | null;
  git: { head: string | null; branch: string | null; changed: string[] } | null;
  updated_at: string;
}

/** Material and the recorded receipt for the configured Wiki. */
export interface WikiArchive {
  root: string;
  prompt: string;
  receipt: { page: string; commit: string | null; recorded_at: string } | null;
}

/** Prompts and a reservation for a manual takeover of this run. */
export interface Handoff {
  job_id: number;
  issue_identifier: string;
  executor: ExecutorKind | null;
  active: boolean;
  context: IssueContext | null;
  memory: RunMemory | null;
  prompt: string;
  review_prompt: string;
  claimed_by: ExecutorKind | null;
  claim_id: string | null;
  archive?: WikiArchive | null;
}

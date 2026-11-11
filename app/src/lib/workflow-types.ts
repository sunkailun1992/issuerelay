import type { Effort, ExecutorKind, PermissionProfile, RunMode } from "./types";

/** The request and references registered for one issue revision. */
export interface IssueContext {
  revision: number;
  origin_executor: ExecutorKind | null;
  preferred_executor: ExecutorKind | null;
  request: string;
  conversation_history?: string | null;
  scope: string;
  acceptance: string[];
  source_refs: string[];
}

/** The exact context and execution settings covered by one dispatch review. */
export interface DispatchPreview {
  token: string;
  issue_uuid: string;
  issue_identifier: string;
  title: string;
  mode: RunMode;
  context: IssueContext | null;
  executor: ExecutorKind;
  directory: string;
  model: string | null;
  effort: Effort | null;
  wiki_root?: string | null;
}

/** The exact source, destination and evidence approved for continuing an interrupted run. */
export interface ContinuationPreview extends DispatchPreview {
  source_job_id: number;
  source_executor: ExecutorKind;
  strategy: "resume_session" | "handoff";
  permission: PermissionProfile;
  claude_ultracode?: boolean;
  evidence: string;
}

/** Recent text from the provider's own session for local review. */
export interface NativeSessionPreview {
  session_id: string;
  title: string | null;
  messages: { role: "user" | "assistant"; text: string }[];
  has_more: boolean;
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

import type { Effort, ExecutorKind, JobState, LeanStatus, PermissionProfile, RunMode } from "./types";

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

export type VerificationStatus = "waiting_binding" | "queued" | "running" | "passed" | "failed" | "blocked";

/** The accepted delivery and the runs that developed or independently checked it. */
export interface AcceptanceLinks {
  development_job_id: number;
  repair_of_job_id: number | null;
  repair_job_id?: number | null;
  verification_job_id: number | null;
  delivery: string | null;
  verification_status: VerificationStatus | null;
  verification_evidence?: string[];
  returned_for_repair: boolean;
  repair_reason: string | null;
}

export type AcceptanceAction =
  | { action: "start_verification"; job_id: number; executor: ExecutorKind; delivery: string }
  | { action: "return_for_repair"; job_id: number; reason: string };

/** The exact action and current delivery facts shown before native confirmation. */
export interface AcceptanceActionPreview {
  token: string;
  action: AcceptanceAction;
  issue_identifier: string;
  job_id: number | null;
  current_state: JobState | null;
  acceptance: {
    source_job_id: number;
    delivery: string | null;
    executor: ExecutorKind | null;
    directory: string | null;
    model: string | null;
    effort: Effort | null;
    permission: string | null;
    lean: LeanStatus | null;
    reason: string | null;
    scope?: string | null;
    /** Missing on hosts predating delivery-bound control previews. */
    delivery_fingerprint?: string;
    linear_updated_at?: string;
    brief_revision?: number | null;
    context?: IssueContext | null;
  } | null;
  expires_in_seconds: number;
}

export interface AcceptanceActionResult {
  action: AcceptanceAction;
  issue_identifier: string;
  job_id: number | null;
  result: "verification_queued" | "returned_for_repair";
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
  acceptance?: AcceptanceLinks | null;
}

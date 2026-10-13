import type { LinearSetup, StatusSnapshot } from "./types";

export type TrayState = "idle" | "running" | "paused" | "problem";

/** Why IssueRelay needs attention; the overview names it in a translated headline. */
export type ProblemCause = "not_configured" | "interface_changed" | "connect_failed" | "check_failed" | "start_failed";

/**
 * The problem rule of the tray (`tray::tray_view` in app/src-tauri/src/tray.rs): not connected
 * to Linear, the last check failed, or a job could not start. While the first connection is
 * still under way (`connecting`) nothing is a problem yet. Both sides are tested with
 * app/src/lib/problem-cases.json.
 */
export function problemCause(status: StatusSnapshot, linear: LinearSetup): ProblemCause | null {
  if (linear.kind === "connecting") return null;
  if (linear.kind !== "connected") return linear.kind;
  if (status.last_poll_at !== null && !status.linear_ok) return "check_failed";
  if (status.last_error !== null) return "start_failed";
  return null;
}

/** A problem wins over paused, which wins over running. */
export function trayState(status: StatusSnapshot, linear: LinearSetup): TrayState {
  if (problemCause(status, linear) !== null) return "problem";
  if (status.paused) return "paused";
  return status.running > 0 ? "running" : "idle";
}

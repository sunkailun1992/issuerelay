import type { UseQueryResult } from "@tanstack/react-query";
import type { DoctorReport, ExecutorReport } from "@/lib/types";

/** The environment check, as the guide's page holds it for the two steps that show it. */
export type DoctorQuery = UseQueryResult<DoctorReport, Error>;

/**
 * Where the check stands. A check that failed is a failure even when an older report is still
 * held (the environment check page does the same); while a new check runs the old report stays.
 */
export type Check =
  | { phase: "failed"; error: unknown }
  | { phase: "loading" }
  | { phase: "ready"; report: DoctorReport; checking: boolean };

export function checkOf(doctor: DoctorQuery): Check {
  if (doctor.isError && !doctor.isFetching) return { phase: "failed", error: doctor.error };
  if (doctor.data === undefined) return { phase: "loading" };
  return { phase: "ready", report: doctor.data, checking: doctor.isFetching };
}

/**
 * Installed, logged in and connected to Linear: the readiness of `ExecutorReport::ready` in core.
 * A check that could not tell leaves a part false, so the executor is not counted.
 */
export function executorReady(report: ExecutorReport): boolean {
  return report.program !== null && report.logged_in && report.linear_mcp;
}

/** The workspace has the label and every status, as far as the check could look. */
export function workspaceComplete(report: DoctorReport): boolean {
  return report.workspace !== null && report.workspace.label_ok && report.workspace.missing_states.length === 0;
}

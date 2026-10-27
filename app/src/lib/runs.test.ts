import { expect, test } from "vitest";
import { DEMO_JOBS } from "./mock-fixtures";
import { isActive, isNewestRun, isPlanned, isPlanRun, newestRuns, parseRunFilter, RUN_FILTERS } from "./runs";
import { JOB_STATES } from "./types";

test("the newest run of an issue is the one with the greatest id", () => {
  // The demo has two runs of DEMO-12: the old failed one (id 2) and the one running now (id 12).
  const ids = newestRuns(DEMO_JOBS).map((job) => job.id).sort((a, b) => a - b);
  expect(ids).toEqual([5, 6, 7, 8, 9, 10, 11, 12]);
});

test("which run is the newest does not depend on the order of the list", () => {
  const reversed = [...DEMO_JOBS].reverse();
  expect(newestRuns(reversed).map((job) => job.id).sort((a, b) => a - b)).toEqual([5, 6, 7, 8, 9, 10, 11, 12]);
});

test("a list without runs has no newest run", () => {
  expect(newestRuns([])).toEqual([]);
});

test("a run is the newest unless a run of the same issue has a greater id", () => {
  const older = DEMO_JOBS.find((job) => job.id === 2)!;
  const newer = DEMO_JOBS.find((job) => job.id === 12)!;
  expect(isNewestRun(older, DEMO_JOBS)).toBe(false);
  expect(isNewestRun(newer, DEMO_JOBS)).toBe(true);
  // A run of another issue never counts.
  expect(isNewestRun(DEMO_JOBS.find((job) => job.id === 8)!, DEMO_JOBS)).toBe(true);
  // Nothing known of the issue's other runs: the run stands.
  expect(isNewestRun(older, [])).toBe(true);
});

test("a run is active while it waits, is queued or runs, like core's JobState::is_active", () => {
  const states = Object.fromEntries(DEMO_JOBS.map((job) => [job.state, isActive(job)]));
  expect(states).toEqual({
    running: true,
    queued: true,
    waiting_binding: true,
    awaiting_acceptance: false,
    needs_info: false,
    failed: false,
    done: false,
    canceled: false,
  });
});

test("the run list can be filtered by \"all\" or by any state of a run, and by nothing else", () => {
  expect([...RUN_FILTERS]).toEqual(["all", ...JOB_STATES]);
});

test("the filter an address asks for is one of those, and anything else is \"all\"", () => {
  for (const filter of RUN_FILTERS) expect(parseRunFilter(filter)).toBe(filter);
  for (const unknown of [null, "", "bogus", "FAILED", "failed ", "failed,done", "__proto__", "toString"]) {
    expect(parseRunFilter(unknown), String(unknown)).toBe("all");
  }
});

test("a run is a planning run by its mode, and one from before planning runs is a usual run", () => {
  const run = DEMO_JOBS[0];
  expect(isPlanRun({ ...run, mode: "plan" })).toBe(true);
  expect(isPlanRun({ ...run, mode: "implement" })).toBe(false);
  expect(isPlanRun({ ...run, mode: undefined })).toBe(false);
});

test("only a planning run that reported a plan and waits on the issue has posted its plan", () => {
  const run = { ...DEMO_JOBS[0], mode: "plan" as const, state: "needs_info" as const, report: "planned" };
  expect(isPlanned(run)).toBe(true);
  // A planning run that asked a question instead, or that failed, has not.
  expect(isPlanned({ ...run, report: "needs_info" })).toBe(false);
  expect(isPlanned({ ...run, state: "failed" })).toBe(false);
  // Nor has a usual run, whatever it reported.
  expect(isPlanned({ ...run, mode: "implement" })).toBe(false);
});

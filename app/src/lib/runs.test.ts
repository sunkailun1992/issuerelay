import { expect, test } from "vitest";
import { DEMO_JOBS } from "./mock-fixtures";
import { isNewestRun, newestRuns } from "./runs";

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

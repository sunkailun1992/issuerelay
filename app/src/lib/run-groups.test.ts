import { expect, test } from "vitest";
import { DEMO_JOBS } from "./mock-fixtures";
import { groupIssueRuns } from "./run-groups";

test("groups attempts by issue UUID with the newest attempt first regardless of input order", () => {
  const original = DEMO_JOBS.map((job) => job.id);
  const groups = groupIssueRuns([...DEMO_JOBS].reverse());
  expect(groups.map((group) => group.latest.id)).toEqual([12, 11, 10, 9, 8, 7, 6, 5]);
  expect(groups.find((group) => group.latest.id === 12)?.runs.map((job) => job.id)).toEqual([12, 2]);
  expect(DEMO_JOBS.map((job) => job.id)).toEqual(original);
  expect(groupIssueRuns([])).toEqual([]);
});

test("identical identifiers from different UUIDs and missing UUIDs never combine", () => {
  const base = DEMO_JOBS[0];
  const groups = groupIssueRuns([
    { ...base, id: 31, issue_uuid: "issue-one", issue_identifier: "DEMO-1" },
    { ...base, id: 32, issue_uuid: "issue-two", issue_identifier: "DEMO-1" },
    { ...base, id: 33, issue_uuid: "", issue_identifier: "DEMO-1" },
    { ...base, id: 34, issue_uuid: " ", issue_identifier: "DEMO-1" },
  ]);
  expect(groups.map((group) => group.runs.map((job) => job.id))).toEqual([[34], [33], [32], [31]]);
  expect(new Set(groups.map((group) => group.key)).size).toBe(4);
});

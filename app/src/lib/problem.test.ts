import { expect, test } from "vitest";
import cases from "./problem-cases.json";
import { problemCause, trayState } from "./problem";
import type { LinearSetup, StatusSnapshot } from "./types";

type Row = (typeof cases)[number];

function inputs(row: Row): [StatusSnapshot, LinearSetup] {
  const setups: Record<string, LinearSetup> = {
    connected: { kind: "connected", method: "oauth" },
    not_configured: { kind: "not_configured" },
    interface_changed: { kind: "interface_changed", message: "list_issues changed" },
    connect_failed: { kind: "connect_failed", message: "network is down" },
    connecting: { kind: "connecting" },
  };
  const status: StatusSnapshot = {
    paused: row.paused,
    linear_ok: row.linear_ok,
    viewer: null,
    last_poll_at: row.checked ? "2026-10-12T08:59:30Z" : null,
    last_error: row.last_error ? "could not start DEMO-1" : null,
    running: row.running,
    queued: 0,
    waiting: 0,
    draining: false,
    held: [],
  };
  return [status, setups[row.linear]];
}

test.each(cases)("$name: the sidebar shows $expected, as the tray does", (row) => {
  expect(trayState(...inputs(row))).toBe(row.expected);
});

test("every problem has a cause the overview can name", () => {
  for (const row of cases) {
    expect(problemCause(...inputs(row)) !== null, row.name).toBe(row.expected === "problem");
  }
});

test("the cause is what went wrong, named by the structured data and not by the engine's text", () => {
  const byName = (name: string) => inputs(cases.find((row) => row.name === name)!);
  expect(problemCause(...byName("not logged in"))).toBe("not_configured");
  expect(problemCause(...byName("Linear's tools changed"))).toBe("interface_changed");
  expect(problemCause(...byName("connecting failed"))).toBe("connect_failed");
  expect(problemCause(...byName("the last check failed"))).toBe("check_failed");
  expect(problemCause(...byName("a job could not start"))).toBe("start_failed");
});

test("a failed check is named before a job that could not start", () => {
  const [status, linear] = inputs(cases.find((row) => row.name === "a job could not start")!);
  expect(problemCause({ ...status, linear_ok: false }, linear)).toBe("check_failed");
});

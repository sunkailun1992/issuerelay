import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useLocation } from "react-router";
import { vi } from "vitest";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { type Backend, BackendError } from "@/lib/backend";
import { MOCK_NOW, setFixedClock } from "@/lib/clock";
import { formatTime } from "@/lib/format";
import { createMockBackend } from "@/lib/mock";
import type { Job, LinearSetup, StatusSnapshot, StatusView } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import Overview from "./Overview";

test("the overview shows what runs, what waits and what needs attention", async () => {
  await renderWithProviders(<Overview />);
  const running = await screen.findByRole("region", { name: "Running" });
  expect(within(running).getByText("DEMO-12")).toBeInTheDocument();
  const queued = screen.getByRole("region", { name: "Queued" });
  expect(queued).toHaveTextContent("DEMO-11");
  expect(queued).toHaveTextContent(`Retrying at ${formatTime("2026-10-12T09:02:00Z", "en")}`);
  // DEMO-8's newest run failed; DEMO-12's old failure was followed by a new run and is not counted.
  expect(await screen.findByText(/1 run failed/)).toBeInTheDocument();
  expect(screen.getByText(/1 issue awaits your acceptance/)).toBeInTheDocument();
  expect(screen.getByText("Jane Doe")).toBeInTheDocument();
});

test("the monitoring switch pauses and resumes", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const pause = vi.spyOn(backend, "pause");
  await renderWithProviders(<Overview />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Monitoring" }));
  expect(pause).toHaveBeenCalled();
});

test("a first run without a login offers the login and the environment check", async () => {
  await renderWithProviders(<Overview />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByRole("button", { name: "Log in to Linear" })).toBeInTheDocument();
  expect(screen.getByText("Nothing is running")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Open the environment check" })).toBeInTheDocument();
});

test("while the first connection is under way the overview says so, then follows it", async () => {
  const backend = createMockBackend("connecting", { latencyMs: 0 });
  await renderWithProviders(
    <>
      <EngineEventsBridge />
      <Overview />
    </>,
    { backend },
  );
  expect(await screen.findByText("Connecting to Linear…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Log in to Linear" })).not.toBeInTheDocument();
  // Connecting is no problem yet: no reminder.
  expect(screen.queryByRole("link", { name: "Open the environment check" })).not.toBeInTheDocument();
  // The connection finishes; the Status event refreshes the card without a reload.
  await act(() => backend.linearReconnect());
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
});

test("an environment problem is named, with the engine's line as a detail", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const view = await backend.status();
  backend.status = vi.fn().mockResolvedValue({
    ...view,
    status: { ...view.status, last_error: "could not start DEMO-11: Linear did not answer" },
  });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("A run could not be started")).toBeInTheDocument();
  expect(screen.getByText("could not start DEMO-11: Linear did not answer")).toBeInTheDocument();
});

test("when the status cannot be read the page says so and offers a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.status = vi.fn().mockRejectedValue(new BackendError("the engine did not answer"));
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("Could not read IssueRelay's status")).toBeInTheDocument();
  expect(screen.getByText("the engine did not answer")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});

/** Shows where the router is, so that a test can follow a link. */
function Where() {
  const location = useLocation();
  return <p data-testid="where">{location.pathname + location.search}</p>;
}

/** The demo backend whose status is `patch` of the demo's. */
async function backendWithStatus(patch: Partial<StatusSnapshot>): Promise<Backend> {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const view = await backend.status();
  backend.status = async (): Promise<StatusView> => ({ ...view, status: { ...view.status, ...patch } });
  return backend;
}

/** The demo backend whose list of runs is what `change` makes of the demo's. */
function backendWithJobs(change: (jobs: Job[]) => Job[]): Backend {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.listJobs.bind(backend);
  backend.listJobs = async (limit) => change(await read(limit));
  return backend;
}

/** A finished run made from the demo's DEMO-7. */
function finishedRun(base: Job, n: number, minutesAgo: number, fields: Partial<Job> = {}): Job {
  return {
    ...base,
    id: 100 + n,
    issue_uuid: `issue-extra-${n}`,
    issue_identifier: `DEMO-${100 + n}`,
    title: `Extra task ${n}`,
    state: "done",
    finished_at: new Date(MOCK_NOW.getTime() - minutesAgo * 60_000).toISOString(),
    summary: `Summary ${n}`,
    ...fields,
  };
}

test("the check button asks for a check", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const poll = vi.spyOn(backend, "pollNow");
  await renderWithProviders(<Overview />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Check now" }));
  expect(poll).toHaveBeenCalledTimes(1);
});

test("the monitoring switch shows the state and follows it", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const resume = vi.spyOn(backend, "resume");
  await renderWithProviders(<Overview />, { backend });
  const monitoring = await screen.findByRole("switch", { name: "Monitoring" });
  expect(monitoring).toBeChecked();
  await userEvent.click(monitoring);
  await waitFor(() => expect(screen.getByRole("switch", { name: "Monitoring" })).not.toBeChecked());
  await userEvent.click(screen.getByRole("switch", { name: "Monitoring" }));
  expect(resume).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(screen.getByRole("switch", { name: "Monitoring" })).toBeChecked());
});

test("a monitor that was paused before shows the switch off", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await backend.pause();
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByRole("switch", { name: "Monitoring" })).not.toBeChecked();
});

test("the reminders lead to where the matter is dealt with", async () => {
  const backend = await backendWithStatus({ last_error: "could not start DEMO-11: Linear did not answer" });
  await renderWithProviders(
    <>
      <Overview />
      <Where />
    </>,
    { backend },
  );
  expect(await screen.findByRole("link", { name: "Open the environment check" })).toHaveAttribute("href", "/doctor");
  // The reminders about runs: failed first, then awaiting acceptance.
  const views = await screen.findAllByRole("link", { name: "View" });
  expect(views.map((link) => link.getAttribute("href"))).toEqual(["/runs?state=failed", "/issues"]);
  await userEvent.click(views[0]);
  expect(screen.getByTestId("where")).toHaveTextContent("/runs?state=failed");
});

test("every reminder has its own wording for the plural", async () => {
  const backend = backendWithJobs((jobs) => {
    const base = jobs.find((job) => job.state === "done")!;
    return [
      ...jobs,
      finishedRun(base, 1, 5, { state: "failed", reason: "boom" }),
      finishedRun(base, 2, 6, { state: "awaiting_acceptance" }),
    ];
  });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("2 runs failed")).toBeInTheDocument();
  expect(screen.getByText("2 issues await your acceptance")).toBeInTheDocument();
});

test("nothing is reported when there is nothing to report", async () => {
  const backend = backendWithJobs(() => []);
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("Nothing is running")).toBeInTheDocument();
  expect(screen.queryAllByRole("alert")).toHaveLength(0);
  expect(screen.queryAllByRole("status")).toHaveLength(0);
});

test("only the environment problem is an alert; the notices about runs and the wait are polite status notices", async () => {
  const backend = await backendWithStatus({ last_error: "could not start DEMO-11: Linear did not answer", draining: true });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("1 run failed")).toBeInTheDocument();
  expect(screen.getByText("A run could not be started").closest("[role=alert]")).not.toBeNull();
  for (const text of ["1 run failed", "1 issue awaits your acceptance", "Waiting for the runs in progress to finish; nothing new starts"]) {
    const notice = screen.getByText(text);
    expect(notice.closest("[role=status]"), text).not.toBeNull();
    expect(notice.closest("[role=alert]"), text).toBeNull();
  }
  // Screen readers announce an alert at once, so only one thing is allowed to be one.
  expect(screen.getAllByRole("alert")).toHaveLength(1);
});

test("the icons of the notices carry their state colour, which the alert's own text-current must not undo", async () => {
  await renderWithProviders(<Overview />);
  const failed = (await screen.findByText("1 run failed")).closest("[data-slot=alert]")!;
  // The alert sets `*:[svg]:text-current`; only an important utility outranks it.
  expect(failed.querySelector("svg")).toHaveClass("text-state-failed!");
  const awaiting = screen.getByText("1 issue awaits your acceptance").closest("[data-slot=alert]")!;
  expect(awaiting.querySelector("svg")).toHaveClass("text-state-awaiting!");
});

test("an acceptance that a newer run of the issue replaced is no reminder", async () => {
  const backend = backendWithJobs((jobs) => {
    const awaiting = jobs.find((job) => job.id === 10)!;
    // DEMO-10 was set to Todo again and picked up anew: its old run no longer waits for acceptance.
    return [...jobs, { ...awaiting, id: 13, state: "queued", summary: null, finished_at: null }];
  });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText(/1 run failed/)).toBeInTheDocument();
  expect(screen.queryByText(/awaits? your acceptance/)).not.toBeInTheDocument();
});

test("a connection that failed is named in a reminder, with Linear's message under it", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const view = await backend.status();
  backend.status = async (): Promise<StatusView> => ({ ...view, linear: { kind: "connect_failed", message: "Linear did not answer within 30 s" } });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("Could not connect to Linear; IssueRelay tries again every minute")).toBeInTheDocument();
  // The card and the reminder both quote what Linear said.
  expect(screen.getAllByText("Linear did not answer within 30 s")).toHaveLength(2);
  expect(screen.getByRole("link", { name: "Open the environment check" })).toHaveAttribute("href", "/doctor");
});

test("a monitor that is waiting for the runs to finish says nothing new starts", async () => {
  const backend = await backendWithStatus({ draining: true });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("Waiting for the runs in progress to finish; nothing new starts")).toBeInTheDocument();
});

test("a failed check names the matter and quotes the engine under it", async () => {
  const backend = await backendWithStatus({ linear_ok: false, last_error: "Linear did not answer within 30 s" });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("The last check of Linear failed")).toBeInTheDocument();
  expect(screen.getByText("Linear did not answer within 30 s")).toBeInTheDocument();
});

test("every kind of environment problem has a headline of its own", async () => {
  const cases: [LinearSetup, Partial<StatusSnapshot>, string][] = [
    [{ kind: "not_configured" }, {}, "Not logged in to Linear"],
    [{ kind: "interface_changed", message: "Linear no longer has the tool `list_issues`" }, {}, "Linear's tools changed: update IssueRelay"],
    [{ kind: "connect_failed", message: "Linear did not answer" }, {}, "Could not connect to Linear; IssueRelay tries again every minute"],
    [{ kind: "connected", method: "oauth" }, { linear_ok: false }, "The last check of Linear failed"],
    [{ kind: "connected", method: "oauth" }, { last_error: "could not start DEMO-11" }, "A run could not be started"],
  ];
  for (const [linear, patch, headline] of cases) {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const view = await backend.status();
    backend.status = async (): Promise<StatusView> => ({ status: { ...view.status, ...patch }, linear });
    const rendered = await renderWithProviders(<Overview />, { backend });
    // The card may say the same; the reminder is the one inside an alert, with the way to the environment check.
    const title = (await screen.findAllByText(headline)).find((text) => text.closest("[role=alert]") !== null);
    expect(title, headline).toBeDefined();
    const reminder = title!.closest<HTMLElement>("[role=alert]")!;
    expect(within(reminder).getByRole("link", { name: "Open the environment check" }), headline).toHaveAttribute("href", "/doctor");
    rendered.unmount();
  }
});

test("the engine's line in a reminder is cleaned of escape sequences", async () => {
  const esc = String.fromCharCode(27);
  const backend = await backendWithStatus({ last_error: `could not start DEMO-11${esc}[31m: down${esc}[0m` });
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("could not start DEMO-11: down")).toBeInTheDocument();
});

test("a running job shows its executor and how long it has run, and counts on", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  try {
    await renderWithProviders(<Overview />);
    const running = await screen.findByRole("region", { name: "Running" });
    expect(running).toHaveTextContent("Add a health check endpoint");
    expect(running).toHaveTextContent("Claude Code");
    // Started at 08:57:37, the demo's clock is 09:00:00.
    expect(running).toHaveTextContent("2 min 23 s");
    setFixedClock(new Date(MOCK_NOW.getTime() + 7_000));
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(running).toHaveTextContent("2 min 30 s");
  } finally {
    vi.useRealTimers();
  }
});

test("the times of the recent results are worked out again while the page stays open", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  try {
    await renderWithProviders(<Overview />);
    const recent = await screen.findByRole("region", { name: "Recent results" });
    expect(recent).toHaveTextContent("20 minutes ago");
    setFixedClock(new Date(MOCK_NOW.getTime() + 5 * 60_000));
    act(() => {
      vi.advanceTimersByTime(30_000);
    });
    expect(recent).toHaveTextContent("25 minutes ago");
  } finally {
    vi.useRealTimers();
  }
});

test("the timer of a running job ends with the page", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  try {
    const { unmount } = await renderWithProviders(<Overview />);
    await screen.findByRole("region", { name: "Running" });
    expect(vi.getTimerCount()).toBeGreaterThan(0);
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  } finally {
    vi.useRealTimers();
  }
});

test("a running job can be stopped from the overview", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const cancel = vi.spyOn(backend, "cancelJob");
  await renderWithProviders(<Overview />, { backend });
  const running = await screen.findByRole("region", { name: "Running" });
  await userEvent.click(within(running).getByRole("button", { name: "Stop" }));
  expect(cancel).toHaveBeenCalledWith(12);
});

test("a running job leads to its run", async () => {
  await renderWithProviders(
    <>
      <Overview />
      <Where />
    </>,
  );
  const running = await screen.findByRole("region", { name: "Running" });
  await userEvent.click(within(running).getByRole("link", { name: /DEMO-12/ }));
  expect(screen.getByTestId("where")).toHaveTextContent("/runs/12");
});

test("the queue lists the queued and the waiting, each in its own words", async () => {
  await renderWithProviders(<Overview />);
  const queue = await screen.findByRole("region", { name: "Queued" });
  const [queued, waiting] = within(queue).getAllByRole("listitem");
  expect(queued).toHaveTextContent("DEMO-11");
  expect(queued).toHaveTextContent("Translate the settings page");
  expect(queued).toHaveTextContent("Queued");
  expect(waiting).toHaveTextContent("DEMO-5");
  expect(waiting).toHaveTextContent("Waiting for setup");
  // The engine's sentence is the detail under the translated state, not the headline.
  expect(within(waiting).getByText("project is not bound to a local directory")).toBeInTheDocument();
});

test("only a queued job that was held back shows when it is tried again", async () => {
  await renderWithProviders(<Overview />);
  const queue = await screen.findByRole("region", { name: "Queued" });
  const retrying = within(queue).getAllByText(/Retrying at/);
  expect(retrying).toHaveLength(1);
  expect(retrying[0].closest("li")).toHaveTextContent("DEMO-11");
});

test("an empty queue says nothing is waiting", async () => {
  await renderWithProviders(<Overview />, { backend: backendWithJobs((jobs) => jobs.filter((job) => !["queued", "waiting_binding"].includes(job.state))) });
  const queue = await screen.findByRole("region", { name: "Queued" });
  expect(within(queue).getByText("Nothing is waiting")).toBeInTheDocument();
  expect(within(queue).queryAllByRole("listitem")).toHaveLength(0);
});

test("the recent results are the finished runs, newest first, each leading to its run", async () => {
  await renderWithProviders(
    <>
      <Overview />
      <Where />
    </>,
  );
  const recent = await screen.findByRole("region", { name: "Recent results" });
  const rows = within(recent).getAllByRole("link");
  // DEMO-10 finished 20 minutes ago, DEMO-9 an hour ago, DEMO-8 70 minutes ago ... and nothing that is still going.
  expect(rows.map((row) => row.getAttribute("href")).slice(0, 3)).toEqual(["/runs/10", "/runs/9", "/runs/8"]);
  expect(rows[0]).toHaveTextContent("Awaiting acceptance");
  expect(rows[0]).toHaveTextContent("DEMO-10");
  expect(rows[0]).toHaveTextContent("Add hello.txt");
  expect(rows[0]).toHaveTextContent("20 minutes ago");
  for (const unfinished of ["DEMO-11", "DEMO-5"]) {
    expect(within(recent).queryByText(unfinished)).not.toBeInTheDocument();
  }
  await userEvent.click(rows[1]);
  expect(screen.getByTestId("where")).toHaveTextContent("/runs/9");
});

test("at most eight finished runs are listed", async () => {
  const backend = backendWithJobs((jobs) => {
    const base = jobs.find((job) => job.state === "done")!;
    return [...jobs, ...Array.from({ length: 10 }, (_, n) => finishedRun(base, n, 1 + n))];
  });
  await renderWithProviders(<Overview />, { backend });
  const recent = await screen.findByRole("region", { name: "Recent results" });
  const rows = within(recent).getAllByRole("link");
  expect(rows).toHaveLength(8);
  // The most recently finished first, whichever was recorded last.
  expect(rows[0]).toHaveTextContent("DEMO-100");
  expect(rows[7]).toHaveTextContent("DEMO-107");
});

test("a run that finished last is first, though it was recorded earlier", async () => {
  const backend = backendWithJobs((jobs) => {
    const base = jobs.find((job) => job.state === "done")!;
    return [...jobs, finishedRun(base, 1, 1), finishedRun(base, 2, 3), { ...finishedRun(base, 0, 2), id: 3 }];
  });
  await renderWithProviders(<Overview />, { backend });
  const recent = await screen.findByRole("region", { name: "Recent results" });
  const order = within(recent).getAllByRole("link").slice(0, 3).map((row) => row.getAttribute("href"));
  expect(order).toEqual(["/runs/101", "/runs/3", "/runs/102"]);
});

test("a finished run shows the first line of its summary, or else of its reason", async () => {
  const backend = backendWithJobs((jobs) => {
    const base = jobs.find((job) => job.state === "done")!;
    return [
      ...jobs,
      finishedRun(base, 1, 1, { summary: `First line of the summary${String.fromCharCode(27)}[1m\nsecond line` }),
      finishedRun(base, 2, 2, { state: "failed", summary: null, reason: "the process was killed\nmore detail" }),
    ];
  });
  await renderWithProviders(<Overview />, { backend });
  const recent = await screen.findByRole("region", { name: "Recent results" });
  expect(within(recent).getByText("First line of the summary")).toBeInTheDocument();
  expect(within(recent).queryByText(/second line/)).not.toBeInTheDocument();
  expect(within(recent).getByText("the process was killed")).toBeInTheDocument();
  expect(within(recent).queryByText(/more detail/)).not.toBeInTheDocument();
  // The demo's failed DEMO-8 has a reason but no summary.
  expect(within(recent).getByText("run exceeded the time limit")).toBeInTheDocument();
});

test("a failed or stopped run says why; the others say what was done", async () => {
  const backend = backendWithJobs((jobs) => {
    const base = jobs.find((job) => job.state === "done")!;
    return [
      ...jobs,
      finishedRun(base, 1, 1, { state: "failed", summary: "The AI's last words", reason: "run exceeded the time limit (60 min)" }),
      finishedRun(base, 2, 2, { state: "canceled", summary: "Half of the change", reason: "stopped by the user (2)" }),
      finishedRun(base, 3, 3, { state: "done", summary: "Added the endpoint", reason: "left over from an earlier try" }),
    ];
  });
  await renderWithProviders(<Overview />, { backend });
  const recent = await screen.findByRole("region", { name: "Recent results" });
  expect(within(recent).getByText("run exceeded the time limit (60 min)")).toBeInTheDocument();
  expect(within(recent).getByText("stopped by the user (2)")).toBeInTheDocument();
  expect(within(recent).getByText("Added the endpoint")).toBeInTheDocument();
  for (const left of ["The AI's last words", "Half of the change", "left over from an earlier try"]) {
    expect(within(recent).queryByText(left), left).not.toBeInTheDocument();
  }
});

test("no finished run yet says so", async () => {
  const backend = backendWithJobs((jobs) => jobs.filter((job) => ["running", "queued"].includes(job.state)));
  await renderWithProviders(<Overview />, { backend });
  const recent = await screen.findByRole("region", { name: "Recent results" });
  expect(within(recent).getByText("No finished runs yet")).toBeInTheDocument();
});

test("runs that cannot be read are one error with a retry, and the rest of the page stays", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.listJobs.bind(backend);
  backend.listJobs = vi.fn().mockRejectedValueOnce(new BackendError("cannot read the job records")).mockImplementation(read);
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("Could not read the runs")).toBeInTheDocument();
  expect(screen.getByText("cannot read the job records")).toBeInTheDocument();
  for (const name of ["Running", "Queued", "Recent results"]) {
    expect(screen.queryByRole("region", { name })).not.toBeInTheDocument();
  }
  // What does not depend on the runs is still there.
  expect(screen.getByText("Jane Doe")).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Monitoring" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("region", { name: "Running" })).toBeInTheDocument();
  expect(screen.queryByText("Could not read the runs")).not.toBeInTheDocument();
});

test("while the runs are read the three blocks are a placeholder", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.listJobs = () => new Promise<Job[]>(() => {});
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Running" })).not.toBeInTheDocument();
});

test("while the status is read the card and the reminders are a placeholder", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.status = () => new Promise<StatusView>(() => {});
  await renderWithProviders(<Overview />, { backend });
  expect(await screen.findByRole("region", { name: "Running" })).toBeInTheDocument();
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.queryByRole("switch")).not.toBeInTheDocument();
});

test("a retry reads the status again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.status.bind(backend);
  backend.status = vi.fn().mockRejectedValueOnce(new BackendError("the engine did not answer")).mockImplementation(read);
  await renderWithProviders(<Overview />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Try again" }));
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  expect(screen.queryByText("Could not read IssueRelay's status")).not.toBeInTheDocument();
});

test("the overview reads in the interface language", async () => {
  await renderWithProviders(<Overview />, { lang: "zh-CN" });
  expect(await screen.findByRole("region", { name: "运行中" })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "排队中" })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "最近结果" })).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "监控" })).toBeInTheDocument();
  expect(await screen.findByText("1 个任务运行失败")).toBeInTheDocument();
  // Times and durations are worded in the interface's language too.
  expect(screen.getByRole("region", { name: "运行中" })).toHaveTextContent("2 分 23 秒");
  expect(screen.getByRole("region", { name: "排队中" })).toHaveTextContent(`将于 ${formatTime("2026-10-12T09:02:00Z", "zh-CN")} 重试`);
  expect(screen.getByRole("region", { name: "最近结果" })).toHaveTextContent("20 分钟前");
});

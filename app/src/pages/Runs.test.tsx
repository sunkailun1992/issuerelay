import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes, useLocation, useNavigationType } from "react-router";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { MOCK_NOW, setFixedClock } from "@/lib/clock";
import { formatTime } from "@/lib/format";
import { createMockBackend } from "@/lib/mock";
import { RUN_FILTERS, type RunFilter } from "@/lib/runs";
import type { Job, JobState } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { advanceUntil } from "@/test/time";
import Runs from "./Runs";

test("runs can be filtered by state", async () => {
  await renderWithProviders(<Runs />);
  expect(await screen.findByText("DEMO-7")).toBeInTheDocument();
  expect(await screen.findByText(`Retrying at ${formatTime("2026-10-12T09:02:00Z", "en")}`)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Failed" }));
  expect(screen.getByText("DEMO-8")).toBeInTheDocument();
  expect(screen.queryByText("DEMO-7")).not.toBeInTheDocument();
});

test("the state in the address selects its tab", async () => {
  await renderWithProviders(<Runs />, { route: "/runs?state=failed" });
  expect(await screen.findByText("DEMO-8")).toBeInTheDocument();
  expect(screen.queryByText("DEMO-7")).not.toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Failed" })).toHaveAttribute("aria-selected", "true");
});

test("when the runs cannot be read the page says so", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.listJobs = vi.fn().mockRejectedValue(new BackendError("cannot read the job records"));
  await renderWithProviders(<Runs />, { backend });
  expect(await screen.findByText("Could not read the runs")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});

// ---- the page's own behaviour ------------------------------------------------------------------

/** The table row of a run, found by the address of the link in it. */
const rowOfRun = (id: number) => {
  const link = screen.getAllByRole("link").find((a) => a.getAttribute("href") === `/runs/${id}`);
  if (link === undefined) throw new Error(`no row for run ${id}`);
  return link.closest("tr")!;
};
/** The lines of text in a cell: a cell of two lines (the identifier and the executor under it) has both. */
function linesOfCell(cell: HTMLElement): string[] {
  const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT);
  const lines: string[] = [];
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const text = node.textContent?.trim() ?? "";
    if (text !== "") lines.push(text);
  }
  return lines;
}
/** What a row says, cell by cell. */
const cellsOf = (row: HTMLElement) => within(row).getAllByRole("cell").map(linesOfCell);

/** Shows where the router is, so that a test can follow a click. */
function Where() {
  const location = useLocation();
  return <p data-testid="where">{location.pathname + location.search}</p>;
}

/** The pages Runs links to, with the router's location shown. */
const pages = (
  <>
    <Routes>
      <Route path="/runs" element={<Runs />} />
      <Route path="/runs/:id" element={<p>the page of a run</p>} />
    </Routes>
    <Where />
  </>
);

test("the filter has a tab for all runs and one for each state, in the interface's words", async () => {
  await renderWithProviders(<Runs />);
  await screen.findByText("DEMO-7");
  expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
    "All",
    "Waiting for setup",
    "Queued",
    "Running",
    "Awaiting acceptance",
    "Needs information",
    "Done",
    "Failed",
    "Canceled",
  ]);
  expect(screen.getByRole("tab", { name: "All" })).toHaveAttribute("aria-selected", "true");
});

test("the list is a table with a column for each thing it says about a run", async () => {
  await renderWithProviders(<Runs />);
  await screen.findByText("DEMO-7");
  expect(screen.getAllByRole("columnheader").map((header) => header.textContent)).toEqual([
    "Status",
    "Issue / Executor",
    "Title",
    "Started / Duration",
    "Note",
  ]);
  // The engine's order is kept: the newest run first (DEMO-12 has two runs, the old failed one last).
  const ids = screen.getAllByRole("row").slice(1).map((row) => within(row).getByRole("link").getAttribute("href"));
  expect(ids).toEqual(["/runs/12", "/runs/11", "/runs/10", "/runs/9", "/runs/8", "/runs/7", "/runs/6", "/runs/5", "/runs/2"]);
});

test("a row says the state, the issue and who ran it, the title, when and for how long, and what came of it", async () => {
  await renderWithProviders(<Runs />);
  await screen.findByText("DEMO-7");
  const retrying = `Retrying at ${formatTime("2026-10-12T09:02:00Z", "en")}`;
  // The demo's clock stands at 09:00 on 2026-10-12.
  expect(cellsOf(rowOfRun(12))).toEqual([
    ["Running"],
    ["DEMO-12", "Claude Code"],
    ["Add a health check endpoint"],
    ["2 minutes ago", "2 min 23 s"],
    [],
  ]);
  expect(cellsOf(rowOfRun(11))).toEqual([["Queued"], ["DEMO-11", "Codex"], ["Translate the settings page"], ["—"], [retrying]]);
  expect(cellsOf(rowOfRun(10))).toEqual([
    ["Awaiting acceptance"],
    ["DEMO-10", "Claude Code"],
    ["Add hello.txt"],
    ["50 minutes ago", "30 min 0 s"],
    ["Added hello.txt, pushed it to the test branch and read the deployment back"],
  ]);
  expect(cellsOf(rowOfRun(8))).toEqual([
    ["Failed"],
    ["DEMO-8", "Claude Code"],
    ["Speed up the search index"],
    ["2 hours ago", "1 h 0 min"],
    ["run exceeded the time limit"],
  ]);
  expect(cellsOf(rowOfRun(7))).toEqual([
    ["Done"],
    ["DEMO-7", "Claude Code"],
    ["Fix the login redirect"],
    ["4 hours ago", "40 min 0 s"],
    ["Fixed the redirect after login and added a test"],
  ]);
  // A run that never started has no executor yet, and nothing to count.
  expect(cellsOf(rowOfRun(5))).toEqual([
    ["Waiting for setup"],
    ["DEMO-5"],
    ["Write the docs landing page"],
    ["—"],
    ["project is not bound to a local directory"],
  ]);
  expect(cellsOf(rowOfRun(6)).at(-1)).toEqual(["stopped by the user"]);
  expect(cellsOf(rowOfRun(2))).toEqual([
    ["Failed"],
    ["DEMO-12", "Claude Code"],
    ["Add a health check endpoint"],
    ["2 days ago", "5 min 0 s"],
    ["executor exited with code 1"],
  ]);
});

test("the note is the summary of a run that did something and the reason of one that did not", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const jobs = await backend.listJobs(100);
  const base = jobs.find((job) => job.id === 8)!;
  // One run for each case: the state, a summary, a reason.
  const made = (id: number, state: JobState, summary: string | null, reason: string | null): Job => ({
    ...base,
    id,
    issue_uuid: `issue-${id}`,
    issue_identifier: `DEMO-${id}`,
    title: `Run ${id}`,
    state,
    summary,
    reason,
  });
  backend.listJobs = vi.fn().mockResolvedValue([
    made(30, "failed", "a summary", "its reason"),
    made(29, "canceled", "a summary", "its reason"),
    made(28, "done", "a summary", "its reason"),
    made(27, "needs_info", null, "its reason"),
    made(26, "done", null, null),
    made(25, "done", "first line\nsecond line", null),
    made(24, "failed", null, "first\nsecond"),
  ]);
  await renderWithProviders(<Runs />, { backend });
  await screen.findByText("DEMO-30");
  const note = (id: number) => cellsOf(rowOfRun(id)).at(-1);
  expect(note(30)).toEqual(["its reason"]);
  expect(note(29)).toEqual(["its reason"]);
  expect(note(28)).toEqual(["a summary"]);
  expect(note(27)).toEqual(["its reason"]);
  expect(note(26)).toEqual([]);
  // Only the first line: the whole text is on the run's page.
  expect(note(25)).toEqual(["first line"]);
  expect(note(24)).toEqual(["first"]);
});

test("the AI's summary and the engine's reason are cleaned before they are shown", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const base = (await backend.listJobs(100)).find((job) => job.id === 8)!;
  const esc = String.fromCharCode(0x1b);
  const rlo = String.fromCharCode(0x202e);
  backend.listJobs = vi.fn().mockResolvedValue([
    { ...base, id: 31, issue_identifier: "DEMO-31", state: "done", reason: null, summary: `${esc}[31mFixed${esc}[0m it${rlo}` },
    { ...base, id: 32, issue_identifier: "DEMO-32", state: "failed", summary: null, reason: `exit${esc}[2K code${rlo} 1` },
  ]);
  await renderWithProviders(<Runs />, { backend });
  await screen.findByText("DEMO-31");
  expect(cellsOf(rowOfRun(31)).at(-1)).toEqual(["Fixed it"]);
  expect(cellsOf(rowOfRun(32)).at(-1)).toEqual(["exit code 1"]);
});

test("a queued run is held back only if the status says so, and a run that is not queued never is", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const status = await backend.status();
  // The status holds back run 12 as well, which is running and so cannot be held.
  const held = [...status.status.held, { job_id: 12, retry_at: "2026-10-12T09:05:00Z" }];
  backend.status = vi.fn().mockResolvedValue({ ...status, status: { ...status.status, held } });
  await renderWithProviders(<Runs />, { backend });
  await screen.findByText(`Retrying at ${formatTime("2026-10-12T09:02:00Z", "en")}`);
  expect(screen.getAllByText(/Retrying at/)).toHaveLength(1);
  expect(cellsOf(rowOfRun(12)).at(-1)).toEqual([]);
});

test("the runs are shown without the retry time when the status cannot be read", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.status = vi.fn().mockRejectedValue(new BackendError("cannot read the status"));
  await renderWithProviders(<Runs />, { backend });
  expect(await screen.findByText("DEMO-11")).toBeInTheDocument();
  expect(screen.queryByText(/Retrying at/)).not.toBeInTheDocument();
  expect(cellsOf(rowOfRun(11)).at(-1)).toEqual([]);
});

/** The tab of each filter, and the demo's runs that it shows (the newest first). */
const TABS: Record<RunFilter, [string, number[]]> = {
  all: ["All", [12, 11, 10, 9, 8, 7, 6, 5, 2]],
  waiting_binding: ["Waiting for setup", [5]],
  queued: ["Queued", [11]],
  running: ["Running", [12]],
  awaiting_acceptance: ["Awaiting acceptance", [10]],
  needs_info: ["Needs information", [9]],
  done: ["Done", [7]],
  failed: ["Failed", [8, 2]],
  canceled: ["Canceled", [6]],
};

/** The addresses of the runs the table lists. */
const listedRuns = () =>
  screen
    .getAllByRole("row")
    .slice(1)
    .map((row) => Number(within(row).getByRole("link").getAttribute("href")!.split("/").at(-1)));

test.each(RUN_FILTERS)("the address ?state=%s selects its tab and shows the runs in that state", async (filter) => {
  await renderWithProviders(<Runs />, { route: `/runs?state=${filter}` });
  const [label, ids] = TABS[filter];
  expect(await screen.findByRole("tab", { name: label, selected: true })).toBeInTheDocument();
  expect(screen.getAllByRole("tab", { selected: true })).toHaveLength(1);
  await screen.findByRole("table");
  expect(listedRuns()).toEqual(ids);
});

test("a state the page does not know in the address means all runs", async () => {
  for (const state of ["bogus", "FAILED", "", "failed,done"]) {
    const view = await renderWithProviders(<Runs />, { route: `/runs?state=${state}` });
    expect(await screen.findByRole("tab", { name: "All", selected: true }), state).toBeInTheDocument();
    await screen.findByRole("table");
    expect(listedRuns(), state).toEqual(TABS.all[1]);
    view.unmount();
  }
});

test("choosing a tab puts its state in the address, and all runs show again with All", async () => {
  await renderWithProviders(pages, { route: "/runs" });
  await screen.findByText("DEMO-7");
  expect(screen.getByTestId("where")).toHaveTextContent(/^\/runs$/);
  await userEvent.click(screen.getByRole("tab", { name: "Done" }));
  expect(screen.getByTestId("where")).toHaveTextContent(/^\/runs\?state=done$/);
  expect(screen.getByText("DEMO-7")).toBeInTheDocument();
  expect(screen.queryByText("DEMO-8")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Queued" }));
  expect(screen.getByTestId("where")).toHaveTextContent(/^\/runs\?state=queued$/);
  expect(screen.getByText("DEMO-11")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "All" }));
  expect(screen.getByTestId("where")).toHaveTextContent(/^\/runs\?state=all$/);
  expect(screen.getByText("DEMO-8")).toBeInTheDocument();
  expect(screen.getByText("DEMO-7")).toBeInTheDocument();
});

/** Shows how the router came to its location: a new step ("PUSH"), or a step replaced ("REPLACE"). */
function How() {
  return <p data-testid="how">{useNavigationType()}</p>;
}

test("choosing a tab replaces the address instead of stacking one more step to go back through", async () => {
  await renderWithProviders(
    <>
      {pages}
      <How />
    </>,
    { route: "/runs" },
  );
  await screen.findByText("DEMO-7");
  await userEvent.click(screen.getByRole("tab", { name: "Done" }));
  expect(screen.getByTestId("how")).toHaveTextContent("REPLACE");
  await userEvent.click(screen.getByRole("tab", { name: "Failed" }));
  expect(screen.getByTestId("how")).toHaveTextContent("REPLACE");
  // Opening a run is a step of its own, which the way back returns from.
  await userEvent.click(screen.getByRole("link", { name: "DEMO-8" }));
  expect(screen.getByTestId("how")).toHaveTextContent("PUSH");
});

test("a row opens the run, from the identifier and from anywhere else in it", async () => {
  const view = await renderWithProviders(pages, { route: "/runs" });
  await screen.findByText("DEMO-8");
  expect(screen.getByRole("link", { name: "DEMO-8" })).toHaveAttribute("href", "/runs/8");
  await userEvent.click(screen.getByRole("link", { name: "DEMO-8" }));
  expect(screen.getByText("the page of a run")).toBeInTheDocument();
  expect(screen.getByTestId("where")).toHaveTextContent(/^\/runs\/8$/);
  view.unmount();

  await renderWithProviders(pages, { route: "/runs" });
  await screen.findByText("DEMO-7");
  await userEvent.click(within(rowOfRun(7)).getByText("Fix the login redirect"));
  expect(screen.getByTestId("where")).toHaveTextContent(/^\/runs\/7$/);
});

test("a click that something has handled already (the link, say) is not handled again by the row", async () => {
  await renderWithProviders(pages, { route: "/runs" });
  await screen.findByText("DEMO-8");
  // Whoever handles a click says so by preventing its default; the link does that when it navigates.
  const link = screen.getByRole("link", { name: "DEMO-8" });
  link.addEventListener("click", (event) => event.preventDefault());
  await userEvent.click(link);
  expect(screen.getByTestId("where")).toHaveTextContent(/^\/runs$/);
});

test("selecting text in a row does not leave the page", async () => {
  await renderWithProviders(pages, { route: "/runs" });
  await screen.findByText("DEMO-8");
  vi.spyOn(window, "getSelection").mockReturnValue({ toString: () => "run exceeded" } as Selection);
  await userEvent.click(within(rowOfRun(8)).getByText("run exceeded the time limit"));
  expect(screen.getByTestId("where")).toHaveTextContent(/^\/runs$/);
});

test("with no runs at all the page says so, and with none in the chosen state it says that", async () => {
  const empty = await renderWithProviders(<Runs />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText("No runs yet")).toBeInTheDocument();
  expect(screen.getByText("A run shows up here when IssueRelay starts work on an issue.")).toBeInTheDocument();
  expect(screen.queryByText("No runs in this state")).not.toBeInTheDocument();
  expect(screen.queryByRole("table")).not.toBeInTheDocument();
  empty.unmount();

  const backend = createMockBackend("demo", { latencyMs: 0 });
  const failedOnly = (await backend.listJobs(100)).filter((job) => job.state === "failed");
  backend.listJobs = vi.fn().mockResolvedValue(failedOnly);
  await renderWithProviders(<Runs />, { backend, route: "/runs?state=done" });
  expect(await screen.findByText("No runs in this state")).toBeInTheDocument();
  expect(screen.getByText("Choose another state to see more runs.")).toBeInTheDocument();
  expect(screen.queryByText("No runs yet")).not.toBeInTheDocument();
  // The tabs stay, to choose another state.
  await userEvent.click(screen.getByRole("tab", { name: "Failed" }));
  expect(await screen.findByText("DEMO-8")).toBeInTheDocument();
});

test("while the runs are being read the page shows placeholders, not an empty list", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.listJobs = vi.fn(() => new Promise<Job[]>(() => {}));
  await renderWithProviders(<Runs />, { backend });
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.queryByText("No runs yet")).not.toBeInTheDocument();
  expect(screen.queryByRole("table")).not.toBeInTheDocument();
});

test("a failed read is reported with its reason, and trying again reads again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.listJobs.bind(backend);
  backend.listJobs = vi.fn().mockRejectedValueOnce(new BackendError("cannot read the job records")).mockImplementation(read);
  await renderWithProviders(<Runs />, { backend });
  expect(await screen.findByText("Could not read the runs")).toBeInTheDocument();
  expect(screen.getByText("cannot read the job records")).toBeInTheDocument();
  expect(screen.queryByRole("tab")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByText("DEMO-7")).toBeInTheDocument();
  expect(screen.queryByText("Could not read the runs")).not.toBeInTheDocument();
});

test("the page is in the language of the interface", async () => {
  await renderWithProviders(<Runs />, { lang: "zh-CN" });
  await screen.findByText("DEMO-7");
  expect(screen.getByRole("tab", { name: "全部" })).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "失败" })).toBeInTheDocument();
  expect(screen.getAllByRole("columnheader").map((header) => header.textContent)).toEqual(["状态", "编号 / 执行者", "标题", "开始时间 / 时长", "说明"]);
  expect(cellsOf(rowOfRun(7))).toEqual([
    ["已完成"],
    ["DEMO-7", "Claude Code"],
    ["Fix the login redirect"],
    ["4 小时前", "40 分 0 秒"],
    ["Fixed the redirect after login and added a test"],
  ]);
});

test("the time of a run still going is counted on, every second", async () => {
  vi.useFakeTimers();
  try {
    await renderWithProviders(<Runs />);
    await advanceUntil(() => screen.queryAllByText("DEMO-12").length > 0);
    expect(cellsOf(rowOfRun(12))[3]).toEqual(["2 minutes ago", "2 min 23 s"]);
    // The clock moves on by five seconds, and the next tick shows it.
    setFixedClock(new Date(MOCK_NOW.getTime() + 5000));
    await act(() => vi.advanceTimersByTimeAsync(1000));
    expect(cellsOf(rowOfRun(12))[3]).toEqual(["2 minutes ago", "2 min 28 s"]);
    // A run that has ended keeps its time.
    expect(cellsOf(rowOfRun(7))[3]).toEqual(["4 hours ago", "40 min 0 s"]);
  } finally {
    vi.useRealTimers();
  }
});

test("how long ago a run started is worked out again as time passes", async () => {
  vi.useFakeTimers();
  try {
    await renderWithProviders(<Runs />);
    await advanceUntil(() => screen.queryAllByText("DEMO-12").length > 0);
    expect(cellsOf(rowOfRun(12))[3][0]).toBe("2 minutes ago");
    setFixedClock(new Date(MOCK_NOW.getTime() + 10 * 60_000));
    await advanceUntil(() => cellsOf(rowOfRun(12))[3][0] !== "2 minutes ago", { step: 5000, limit: 60_000 });
    expect(cellsOf(rowOfRun(12))[3][0]).toBe("12 minutes ago");
  } finally {
    vi.useRealTimers();
  }
});

test("a list as long as the panel reads says that only the latest runs are shown", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const [newest] = await backend.listJobs(1);
  backend.listJobs = vi.fn(async (limit: number) => Array.from({ length: limit }, (_, n) => ({ ...newest, id: 1000 - n })));
  await renderWithProviders(<Runs />, { backend });
  expect(await screen.findByText("Showing the latest 200 runs; older ones are not listed.")).toBeInTheDocument();
});

test("a list shorter than that says nothing about its length", async () => {
  await renderWithProviders(<Runs />);
  await screen.findByText("DEMO-7");
  expect(screen.queryByText(/Showing the latest/)).not.toBeInTheDocument();
});

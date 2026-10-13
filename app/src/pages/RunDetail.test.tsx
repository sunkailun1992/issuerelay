import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Link, Route, Routes } from "react-router";
import { vi } from "vitest";
import { queryKeys } from "@/hooks/queries";
import { type Backend, BackendError } from "@/lib/backend";
import { MOCK_NOW, setFixedClock } from "@/lib/clock";
import { formatDateTime } from "@/lib/format";
import { createMockBackend } from "@/lib/mock";
import { ago } from "@/lib/mock-fixtures";
import type { Job } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { advanceUntil } from "@/test/time";
import RunDetail from "./RunDetail";

const page = <Routes><Route path="/runs/:id" element={<RunDetail />} /></Routes>;

async function open(state: string, backend = createMockBackend("demo", { latencyMs: 0 })) {
  const job = (await backend.listJobs(100)).find((j) => j.state === state)!;
  await renderWithProviders(page, { backend, route: `/runs/${job.id}` });
  return { backend, job };
}

test("a running job shows its timeline, its raw log and its Linear link", async () => {
  const { job } = await open("running");
  expect(await screen.findByRole("heading", { name: new RegExp(job.issue_identifier) })).toBeInTheDocument();
  expect(await screen.findByText(/Session started/)).toBeInTheDocument();
  expect(await screen.findByRole("button", { name: "Open in Linear" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  expect(screen.getByRole("button", { name: "Copy all" })).toBeInTheDocument();
});

test("a failed job says why under a translated headline", async () => {
  await open("failed");
  expect(await screen.findByText("The run failed")).toBeInTheDocument();
  expect(screen.getByText("run exceeded the time limit")).toBeInTheDocument();
});

test("the page works when the issue list cannot be read, just without the link", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.issues = vi.fn().mockRejectedValue(new BackendError("not connected to Linear: log in first"));
  const { job } = await open("running", backend);
  expect(await screen.findByRole("heading", { name: new RegExp(job.issue_identifier) })).toBeInTheDocument();
  expect(await screen.findByText(/Session started/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Open in Linear" })).not.toBeInTheDocument();
});

test("a run that does not exist says so", async () => {
  await renderWithProviders(page, { route: "/runs/999" });
  expect(await screen.findByText("This run does not exist")).toBeInTheDocument();
});

test("when the run cannot be read the page says so and offers a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.job = vi.fn().mockRejectedValue(new BackendError("cannot read the job records"));
  await renderWithProviders(page, { backend, route: "/runs/12" });
  expect(await screen.findByText("Could not read the run")).toBeInTheDocument();
  expect(screen.getByText("cannot read the job records")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});

// ---- helpers -----------------------------------------------------------------------------------

const ESC = String.fromCharCode(0x1b);
const RLO = String.fromCharCode(0x202e);

const assistant = (...content: unknown[]) => JSON.stringify({ type: "assistant", message: { content } });
const toolResults = (...content: unknown[]) => JSON.stringify({ type: "user", message: { content } });
const completed = (item: object) => JSON.stringify({ type: "item.completed", item });

/** The demo backend whose logs are `lines` (whatever run is asked for). */
function backendWithLog(lines: string[], backend = createMockBackend("demo", { latencyMs: 0 })): Backend {
  backend.jobLog = vi.fn().mockResolvedValue(lines);
  return backend;
}

/** Opens the page of run `id` and waits until the run itself has been read (the demo's issues are all DEMO-n). */
async function openRun(id: number, backend: Backend = createMockBackend("demo", { latencyMs: 0 })) {
  const view = await renderWithProviders(page, { backend, route: `/runs/${id}` });
  await screen.findByRole("heading", { level: 1, name: /^DEMO-/ });
  return view;
}

/** What the card of the run says for a field: the text of the value next to the field's name. */
function field(name: string): string {
  const label = screen.getByText(name, { selector: "dt" });
  return label.parentElement!.querySelector("dd")!.textContent!;
}

/** The card of a tool call or command: the button that opens it. */
const cardOf = (name: string | RegExp) => screen.getByRole("button", { name });

/** The clipboard of the test: what is written to it. Removed again after each test. */
function stubClipboard(writeText = vi.fn().mockResolvedValue(undefined)) {
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  return writeText;
}
afterEach(() => {
  Reflect.deleteProperty(navigator, "clipboard");
});

// ---- the head of the page ---------------------------------------------------------------------

test("the title of the page is the issue's identifier and title", async () => {
  await openRun(12);
  expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("DEMO-12 Add a health check endpoint");
});

test("the card of a run that is going says its state, executor, folder, times and that it has no summary yet", async () => {
  await openRun(12);
  expect(field("Status")).toBe("Running");
  expect(field("Executor")).toBe("Claude Code");
  expect(field("Folder")).toBe("~/code/demo-app");
  expect(field("Created")).toBe(formatDateTime(ago(12 * 60), "en"));
  expect(field("Started")).toBe(formatDateTime("2026-10-12T08:57:37Z", "en"));
  expect(field("Finished")).toBe("—");
  expect(field("Duration")).toBe("2 min 23 s");
  expect(field("Summary")).toBe("—");
});

test("the card of a finished run has its end and its length, and the AI's summary, cleaned", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const done = (await backend.job(7))!;
  backend.job = vi.fn().mockResolvedValue({ ...done, summary: `${ESC}[1mFixed${ESC}[0m it${RLO}\nand tested it` });
  await openRun(7, backend);
  expect(field("Status")).toBe("Done");
  expect(field("Finished")).toBe(formatDateTime(ago(200), "en"));
  expect(field("Duration")).toBe("40 min 0 s");
  expect(field("Summary")).toBe("Fixed it\nand tested it");
});

test("a run that has not started has nothing to say for the fields it has no value for", async () => {
  await openRun(5);
  expect(field("Status")).toBe("Waiting for setup");
  expect(field("Executor")).toBe("—");
  expect(field("Folder")).toBe("—");
  expect(field("Started")).toBe("—");
  expect(field("Finished")).toBe("—");
  expect(field("Duration")).toBe("—");
  // Nothing to copy without a folder.
  expect(screen.queryByRole("button", { name: "Copy" })).not.toBeInTheDocument();
});

test("the folder of the run can be copied", async () => {
  const writeText = stubClipboard();
  await openRun(12);
  await userEvent.click(screen.getByRole("button", { name: "Copy" }));
  expect(writeText).toHaveBeenCalledWith("~/code/demo-app");
  expect(await screen.findByText("Copied")).toBeInTheDocument();
});

test("the folder is shown and copied cleaned, like every other text that is not the interface's own", async () => {
  const writeText = stubClipboard();
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const running = (await backend.job(12))!;
  backend.job = vi.fn().mockResolvedValue({ ...running, directory: `~/code/${ESC}[31mdemo${RLO}-app` });
  await openRun(12, backend);
  expect(field("Folder")).toBe("~/code/demo-app");
  await userEvent.click(screen.getByRole("button", { name: "Copy" }));
  expect(writeText).toHaveBeenCalledWith("~/code/demo-app");
});

test("the duration of a run that is going is counted on", async () => {
  vi.useFakeTimers();
  try {
    await renderWithProviders(page, { route: "/runs/12" });
    await advanceUntil(() => screen.queryByText("Duration") !== null);
    expect(field("Duration")).toBe("2 min 23 s");
    setFixedClock(new Date(MOCK_NOW.getTime() + 7000));
    await act(() => vi.advanceTimersByTimeAsync(1000));
    expect(field("Duration")).toBe("2 min 30 s");
  } finally {
    vi.useRealTimers();
  }
});

// ---- why a run ended as it did ----------------------------------------------------------------

test.each([
  [8, "The run failed", "run exceeded the time limit"],
  [6, "The run was canceled", "stopped by the user"],
  [5, "The run waits for setup", "project is not bound to a local directory"],
])("run %i says under a headline of its own why it is where it is", async (id, headline, reason) => {
  await openRun(id);
  expect(screen.getByText(headline)).toBeInTheDocument();
  expect(screen.getByText(reason)).toBeInTheDocument();
  // The engine's words are never shown without the translated headline above them.
  const note = screen.getByText(reason).closest("[data-slot=alert]") as HTMLElement;
  expect(within(note).getByText(headline)).toBeInTheDocument();
});

test("a reason in any other state is a note from IssueRelay", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const asking = (await backend.job(9))!;
  backend.job = vi.fn().mockResolvedValue({ ...asking, reason: "the AI asked a question" });
  await openRun(9, backend);
  expect(screen.getByText("A note from IssueRelay")).toBeInTheDocument();
  expect(screen.getByText("the AI asked a question")).toBeInTheDocument();
});

test("a run without a reason has no headline for one", async () => {
  await openRun(7);
  for (const headline of ["The run failed", "The run was canceled", "The run waits for setup", "A note from IssueRelay"]) {
    expect(screen.queryByText(headline), headline).not.toBeInTheDocument();
  }
});

test("a failure is announced as an alert, the other notes as notices", async () => {
  const failed = await openRun(8);
  expect(screen.getByRole("alert")).toHaveTextContent("The run failed");
  failed.unmount();
  await openRun(6);
  await screen.findByText("The log of this run was removed by the log cleanup");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("The run was canceled");
});

test("a reason with nothing in it to read is no reason: there is no headline over nothing", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const failed = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({ ...failed, reason: `  ${String.fromCharCode(0x200b)}\n ${ESC}[2K` });
  await openRun(8, backend);
  await screen.findByText("The log of this run was removed by the log cleanup");
  expect(screen.queryByText("The run failed")).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

test("the engine's reason is cleaned before it is shown", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const failed = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({ ...failed, reason: `exit${ESC}[2K code${RLO} 1` });
  await openRun(8, backend);
  expect(screen.getByText("exit code 1")).toBeInTheDocument();
});

// ---- what can be done -------------------------------------------------------------------------

test("a run offers what its state allows, beside the title", async () => {
  await openRun(10);
  expect(screen.getByRole("button", { name: "Mark done" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
});

test("the newest run of an issue can be run again after a failure, an older one cannot", async () => {
  const view = await openRun(8);
  expect(await screen.findByRole("button", { name: "Run again" })).toBeInTheDocument();
  view.unmount();
  // Run 2 is DEMO-12's earlier failure; run 12 is the newest run of that issue now.
  await openRun(2);
  await waitFor(() => expect(screen.queryByRole("button", { name: "Run again" })).not.toBeInTheDocument());
  expect(screen.getByText("The run failed")).toBeInTheDocument();
});

test("marking a run done asks first, from the page of the run", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const markDone = vi.spyOn(backend, "markDone");
  await openRun(10, backend);
  await userEvent.click(screen.getByRole("button", { name: "Mark done" }));
  expect(markDone).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(markDone).toHaveBeenCalledWith(10);
});

test("the issue's link opens in Linear through the backend, with the link of this run's issue", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const openLinear = vi.spyOn(backend, "openLinear");
  await openRun(7, backend);
  await userEvent.click(await screen.findByRole("button", { name: "Open in Linear" }));
  expect(openLinear).toHaveBeenCalledWith("https://linear.app/demo/issue/DEMO-7");
});

test("a run whose issue is not in the list has no link to Linear", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const done = (await backend.job(7))!;
  backend.job = vi.fn().mockResolvedValue({ ...done, issue_uuid: "issue-unknown" });
  const issues = vi.spyOn(backend, "issues");
  await openRun(7, backend);
  await waitFor(() => expect(issues).toHaveBeenCalled());
  await waitFor(() => expect(screen.queryByRole("button", { name: "Open in Linear" })).not.toBeInTheDocument());
  expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();
});

// ---- the timeline -----------------------------------------------------------------------------

test("the timeline of the demo's run: the session, what the AI said, the command and a warning", async () => {
  await openRun(12);
  expect(await screen.findByText("Session started")).toBeInTheDocument();
  expect(screen.getByText("claude-sonnet-5-5")).toBeInTheDocument();
  expect(screen.getByText("I will add the endpoint and a test for it.")).toBeInTheDocument();
  expect(cardOf(/cargo test/)).toBeInTheDocument();
  expect(within(screen.getByRole("group", { name: "stderr" })).getByText("warning: unused import")).toBeInTheDocument();
});

test("the timeline is the tab that is open first, and the raw log is another tab", async () => {
  await openRun(12);
  expect(screen.getByRole("tab", { name: "Timeline" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tab", { name: "Raw log" })).toHaveAttribute("aria-selected", "false");
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  expect(screen.getByRole("tab", { name: "Raw log" })).toHaveAttribute("aria-selected", "true");
  expect(screen.queryByText("Session started")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Timeline" }));
  expect(await screen.findByText("Session started")).toBeInTheDocument();
});

const MIXED_LOG = [
  JSON.stringify({ type: "system", subtype: "init", model: "claude-opus-5-5" }),
  assistant({ type: "text", text: "Looking at it." }, { type: "tool_use", id: "t1", name: "Read", input: { file_path: "src/lib.rs" } }),
  toolResults({ type: "tool_result", tool_use_id: "t1", content: "fn main() {}" }),
  assistant({ type: "tool_use", id: "t2", name: "Bash", input: { command: "cargo test" } }),
  toolResults({ type: "tool_result", tool_use_id: "t2", content: "1 failed", is_error: true }),
  JSON.stringify({ stream: "stderr", text: "warning: slow" }),
  JSON.stringify({ stream: "runner", text: "spawn failed: nope" }),
  JSON.stringify({ type: "result", result: "Could not fix it.", is_error: true }),
];

test("tools and commands are cards that open to show what went in and what came out", async () => {
  await openRun(12, backendWithLog(MIXED_LOG));
  const read = await screen.findByRole("button", { name: "Read" });
  const test = cardOf(/cargo test/);
  expect(read).toHaveAttribute("aria-expanded", "false");
  expect(test).toHaveAttribute("aria-expanded", "false");
  // Closed: only the names are there.
  expect(screen.queryByText(/fn main/)).not.toBeInTheDocument();
  expect(screen.queryByText("1 failed")).not.toBeInTheDocument();
  await userEvent.click(read);
  expect(read).toHaveAttribute("aria-expanded", "true");
  // The input, then the result below it.
  expect(screen.getByText(/fn main/).textContent).toBe('{"file_path":"src/lib.rs"}\n\nfn main() {}');
  await userEvent.click(test);
  // A command's card shows its output; the command itself is in the header.
  expect(screen.getByText("1 failed")).toBeInTheDocument();
  await userEvent.click(read);
  expect(screen.queryByText(/fn main/)).not.toBeInTheDocument();
});

test("a failure is marked in words as well as in colour", async () => {
  await openRun(12, backendWithLog(MIXED_LOG));
  const read = await screen.findByRole("button", { name: "Read" });
  expect(within(read).queryByRole("img", { name: "Failed" })).not.toBeInTheDocument();
  expect(within(cardOf(/cargo test/)).getByRole("img", { name: "Failed" })).toBeInTheDocument();
  // The result of the run failed too.
  expect(within(screen.getByRole("group", { name: "Result" })).getByRole("img", { name: "Failed" })).toBeInTheDocument();
});

test("a result that did not fail has no failure mark", async () => {
  await openRun(12, backendWithLog([JSON.stringify({ type: "result", result: "All good." })]));
  const result = await screen.findByRole("group", { name: "Result" });
  expect(within(result).getByText("All good.")).toBeInTheDocument();
  expect(within(result).queryByRole("img", { name: "Failed" })).not.toBeInTheDocument();
});

test("the rows of the timeline come in the order of the log, and the result is last", async () => {
  await openRun(12, backendWithLog(MIXED_LOG));
  await screen.findByRole("button", { name: "Read" });
  const text = screen.getByRole("tabpanel").textContent ?? "";
  const parts = ["claude-opus-5-5", "Looking at it.", "Read", "cargo test", "warning: slow", "spawn failed: nope", "Could not fix it."];
  const positions = parts.map((part) => text.indexOf(part));
  expect(positions.every((position) => position >= 0)).toBe(true);
  expect(positions).toEqual([...positions].sort((a, b) => a - b));
});

test("lines of IssueRelay's own are labelled as such, beside the executor's stderr", async () => {
  await openRun(12, backendWithLog(MIXED_LOG));
  const note = await screen.findByRole("group", { name: "IssueRelay" });
  expect(within(note).getByText("spawn failed: nope")).toBeInTheDocument();
  const warning = screen.getByRole("group", { name: "stderr" });
  expect(within(warning).getByText("warning: slow")).toBeInTheDocument();
});

test("Codex's errors are rows of their own, and an end in Claude's dialect is not read in a Codex run", async () => {
  const log = [
    completed({ type: "agent_message", text: "Working on it." }),
    JSON.stringify({ type: "error", message: "stream disconnected" }),
    JSON.stringify({ type: "result", result: "Finished fine." }),
  ];
  // Run 11 is a Codex run.
  await openRun(11, backendWithLog(log));
  expect(await screen.findByText("Working on it.")).toBeInTheDocument();
  expect(within(screen.getByRole("group", { name: "Error" })).getByText("stream disconnected")).toBeInTheDocument();
  expect(screen.queryByText("Finished fine.")).not.toBeInTheDocument();
});

test("a run's answer is shown with its label, in full, lines kept", async () => {
  const log = [JSON.stringify({ type: "result", result: "Done.\n\n- one\n- two" })];
  await openRun(12, backendWithLog(log));
  const result = await screen.findByRole("group", { name: "Result" });
  expect(within(result).getByText(/Done\./).textContent).toBe("Done.\n\n- one\n- two");
});

test("a command with several lines is shown whole once its card is open", async () => {
  const command = "git commit -m 'feat: x\n\nwhy'";
  const log = [assistant({ type: "tool_use", id: "t", name: "Bash", input: { command } }), toolResults({ type: "tool_result", tool_use_id: "t", content: "ok" })];
  await openRun(12, backendWithLog(log));
  const card = await screen.findByRole("button", { name: /git commit/ });
  // The whole command is in the card; only its look (one line with an ellipsis, or all of it) changes when it opens.
  expect(card.textContent).toBe(command);
  expect(card).toHaveAttribute("data-state", "closed");
  await userEvent.click(card);
  expect(card).toHaveAttribute("data-state", "open");
  expect(screen.getByText("ok")).toBeInTheDocument();
});

test("a long command opens even without output, to be read whole", async () => {
  const command = `echo ${"long ".repeat(30)}`.trim();
  const log = [assistant({ type: "tool_use", id: "t", name: "Bash", input: { command } })];
  await openRun(12, backendWithLog(log));
  const card = await screen.findByRole("button", { name: /echo long/ });
  expect(card).toHaveAttribute("aria-expanded", "false");
  await userEvent.click(card);
  expect(card).toHaveAttribute("aria-expanded", "true");
});

test("a short command with nothing to show is a card that does not open", async () => {
  const log = [assistant({ type: "tool_use", id: "t", name: "Bash", input: { command: "true" } }), toolResults({ type: "tool_result", tool_use_id: "t", content: "" })];
  await openRun(12, backendWithLog(log));
  expect(await screen.findByText("true")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "true" })).not.toBeInTheDocument();
});

test("what the AI and the executor wrote is shown as text, cleaned, never as markup", async () => {
  const markup = '<img src=x onerror="alert(1)"> and <b>bold</b>';
  const log = [
    assistant({ type: "text", text: `${markup}${ESC}[2J${RLO}` }),
    assistant({ type: "tool_use", id: "t", name: `Re${ESC}[31mad`, input: { file_path: "a.rs" } }),
    toolResults({ type: "tool_result", tool_use_id: "t", content: `${ESC}[32mgreen${ESC}[0m output${RLO}` }),
    JSON.stringify({ stream: "stderr", text: `${ESC}[31mred${ESC}[0m` }),
  ];
  const { container } = await openRun(12, backendWithLog(log));
  expect(await screen.findByText(markup)).toBeInTheDocument();
  expect(container.querySelector("img, b")).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Read" }));
  expect(screen.getByText(/green output/).textContent).toBe('{"file_path":"a.rs"}\n\ngreen output');
  expect(screen.getByText("red")).toBeInTheDocument();
  expect(document.body.textContent).not.toContain(ESC);
  expect(document.body.textContent).not.toContain(RLO);
});

// ---- the log: reading it, copying it, finding it gone ------------------------------------------

test("the raw log is every line of the log as it is, cleaned, in one scrolling box", async () => {
  const log = [
    JSON.stringify({ type: "system", subtype: "init", model: "m" }),
    `plain ${ESC}[31mred${ESC}[0m line${RLO}`,
    "",
    JSON.stringify({ stream: "stderr", text: "warning" }),
  ];
  await openRun(12, backendWithLog(log));
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  const lines = await screen.findByText(/plain red line/);
  expect(lines.textContent).toBe(['{"type":"system","subtype":"init","model":"m"}', "plain red line", "", '{"stream":"stderr","text":"warning"}'].join("\n"));
  expect(document.querySelector("[data-slot=scroll-area]")).toContainElement(lines);
});

test("Copy all puts the cleaned lines on the clipboard and says it did", async () => {
  const log = [`first ${ESC}[31mred${ESC}[0m`, "second", `third${RLO}`];
  const writeText = stubClipboard();
  await openRun(12, backendWithLog(log));
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  await userEvent.click(await screen.findByRole("button", { name: "Copy all" }));
  expect(writeText).toHaveBeenCalledTimes(1);
  expect(writeText).toHaveBeenCalledWith("first red\nsecond\nthird");
  expect(await screen.findByText("Copied")).toBeInTheDocument();
});

test("when copying fails the user is told, with the reason", async () => {
  stubClipboard(vi.fn().mockRejectedValue(new Error("clipboard access was denied")));
  await openRun(12, backendWithLog(["one line"]));
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  await userEvent.click(await screen.findByRole("button", { name: "Copy all" }));
  expect(await screen.findByText("Could not copy")).toBeInTheDocument();
  expect(screen.getByText("clipboard access was denied")).toBeInTheDocument();
  expect(screen.queryByText("Copied")).not.toBeInTheDocument();
});

test("a window with no clipboard at all is also told so, not left clicking", async () => {
  await openRun(12, backendWithLog(["one line"]));
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  await userEvent.click(await screen.findByRole("button", { name: "Copy all" }));
  expect(await screen.findByText("Could not copy")).toBeInTheDocument();
});

test("the log of a run that ended and started is said to be gone when it reads as empty, in both tabs", async () => {
  await openRun(8);
  expect(await screen.findByText("The log of this run was removed by the log cleanup")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  expect(screen.getByText("The log of this run was removed by the log cleanup")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Copy all" })).not.toBeInTheDocument();
});

test("a run that never started never had a log to remove", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const canceled = (await backend.job(6))!;
  backend.job = vi.fn().mockResolvedValue({ ...canceled, started_at: null, executor: null });
  await openRun(6, backend);
  expect(await screen.findByText("Nothing to show yet")).toBeInTheDocument();
  expect(screen.queryByText("The log of this run was removed by the log cleanup")).not.toBeInTheDocument();
});

test("a run that is going and has written nothing yet has nothing to show yet", async () => {
  await openRun(12, backendWithLog([]));
  expect(await screen.findByText("Nothing to show yet")).toBeInTheDocument();
  expect(screen.queryByText("The log of this run was removed by the log cleanup")).not.toBeInTheDocument();
});

test("a log with events but none worth a row has nothing to show in the timeline, and all of it in the raw log", async () => {
  const log = [JSON.stringify({ type: "thread.started", thread_id: "t" }), JSON.stringify({ type: "turn.started" })];
  await openRun(11, backendWithLog(log));
  expect(await screen.findByText("Nothing to show yet")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  expect(screen.getByText(/thread\.started/)).toBeInTheDocument();
  expect(screen.queryByText("Nothing to show yet")).not.toBeInTheDocument();
});

test("while the log is being read the tabs show placeholders", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.jobLog = vi.fn(() => new Promise<string[]>(() => {}));
  await openRun(12, backend);
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.queryByText("Nothing to show yet")).not.toBeInTheDocument();
  // The rest of the page does not wait for the log.
  expect(field("Status")).toBe("Running");
});

test("when the log cannot be read the page goes on without it and says so where the log is", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.jobLog.bind(backend);
  backend.jobLog = vi.fn().mockRejectedValueOnce(new BackendError("cannot read /logs/12.jsonl")).mockImplementation(read);
  await openRun(12, backend);
  expect(await screen.findByText("Could not read the log")).toBeInTheDocument();
  expect(screen.getByText("cannot read /logs/12.jsonl")).toBeInTheDocument();
  // The run itself is there: title, card and actions.
  expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("DEMO-12");
  expect(field("Status")).toBe("Running");
  expect(screen.getByRole("button", { name: "Stop" })).toBeInTheDocument();
  expect(screen.queryByText("Could not read the run")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  expect(screen.getByText("Could not read the log")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByText(/"subtype":"init"/)).toBeInTheDocument();
  expect(screen.queryByText("Could not read the log")).not.toBeInTheDocument();
});

// ---- a run still going: its log is read again --------------------------------------------------

test("the log of a run that is going is read again and shows what came since", async () => {
  vi.useFakeTimers();
  try {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const first = [JSON.stringify({ type: "system", subtype: "init", model: "m" })];
    const later = [...first, assistant({ type: "text", text: "A new thought." })];
    const read = vi.fn().mockResolvedValueOnce(first).mockResolvedValue(later);
    backend.jobLog = read;
    await renderWithProviders(page, { backend, route: "/runs/12" });
    await advanceUntil(() => screen.queryByText("Session started") !== null);
    expect(screen.queryByText("A new thought.")).not.toBeInTheDocument();
    await advanceUntil(() => screen.queryByText("A new thought.") !== null);
    expect(read.mock.calls.length).toBeGreaterThanOrEqual(2);
  } finally {
    vi.useRealTimers();
  }
});

test("the log of a run that has ended is read once", async () => {
  vi.useFakeTimers();
  try {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const read = vi.fn().mockResolvedValue([assistant({ type: "text", text: "Finished." })]);
    backend.jobLog = read;
    await renderWithProviders(page, { backend, route: "/runs/10" });
    await advanceUntil(() => screen.queryByText("Finished.") !== null);
    const calls = read.mock.calls.length;
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(read.mock.calls.length).toBe(calls);
  } finally {
    vi.useRealTimers();
  }
});

/** Stands in for the layout jsdom does not have: a box of this size whose scroll position can be set and read. */
function boxOf(viewport: HTMLElement, size: { height: number; visible: number }) {
  const box = { top: 0, ...size };
  Object.defineProperty(viewport, "scrollHeight", { configurable: true, get: () => box.height });
  Object.defineProperty(viewport, "clientHeight", { configurable: true, get: () => box.visible });
  Object.defineProperty(viewport, "scrollTop", {
    configurable: true,
    get: () => box.top,
    set: (value: number) => {
      box.top = value;
    },
  });
  return box;
}

/** The scroll position the reader chose: sets it and tells the page, as the browser does. */
function scrollTo(viewport: HTMLElement, box: { top: number }, top: number) {
  box.top = top;
  act(() => {
    viewport.dispatchEvent(new Event("scroll"));
  });
}

test("the raw log of a run that is going follows the end, until the reader scrolls up, and again once back at the end", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let lines = ["line 1", "line 2"];
  backend.jobLog = vi.fn(async () => lines);
  const { client } = await openRun(12, backend);
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  await screen.findByText(/line 2/);
  const viewport = document.querySelector<HTMLElement>("[data-slot=scroll-area-viewport]")!;
  const box = boxOf(viewport, { height: 1000, visible: 200 });

  /** The log grows: the page reads it again and shows it. */
  const grow = async (line: string, height: number) => {
    lines = [...lines, line];
    box.height = height;
    await act(() => client.invalidateQueries({ queryKey: queryKeys.log(12) }));
    await screen.findByText(new RegExp(line));
  };

  await grow("line 3", 1100);
  expect(box.top).toBe(1100);
  // The reader goes further up: the page stays where they are.
  scrollTo(viewport, box, 100);
  await grow("line 4", 1200);
  expect(box.top).toBe(100);
  // Back near the end (within a few pixels counts): following again.
  scrollTo(viewport, box, 1200 - 200 - 3);
  await grow("line 5", 1300);
  expect(box.top).toBe(1300);
});

test("the raw log of a run that has ended is left where the reader is", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let lines = ["line 1", "line 2"];
  backend.jobLog = vi.fn(async () => lines);
  const { client } = await openRun(10, backend);
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  await screen.findByText(/line 2/);
  const viewport = document.querySelector<HTMLElement>("[data-slot=scroll-area-viewport]")!;
  const box = boxOf(viewport, { height: 1000, visible: 200 });
  lines = [...lines, "line 3"];
  box.height = 1100;
  await act(() => client.invalidateQueries({ queryKey: queryKeys.log(10) }));
  await screen.findByText(/line 3/);
  expect(box.top).toBe(0);
});

// ---- the page's own states --------------------------------------------------------------------

test("while the run is being read the page shows placeholders under a plain title", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.job = vi.fn(() => new Promise<Job | null>(() => {}));
  await renderWithProviders(page, { backend, route: "/runs/12" });
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Runs");
  expect(screen.queryByText("This run does not exist")).not.toBeInTheDocument();
});

test("a run that does not exist offers the way back to the list of runs", async () => {
  await renderWithProviders(
    <Routes>
      <Route path="/runs" element={<p>the list of runs</p>} />
      <Route path="/runs/:id" element={<RunDetail />} />
    </Routes>,
    { route: "/runs/999" },
  );
  expect(await screen.findByText("This run does not exist")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Back to the runs" }));
  expect(screen.getByText("the list of runs")).toBeInTheDocument();
});

test.each(["abc", "0", "-3", "1.5", "99999999999999999999", "12abc", "012"])(
  "an address with %s for a run asks the backend nothing and says there is no such run",
  async (id) => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const job = vi.spyOn(backend, "job");
    const log = vi.spyOn(backend, "jobLog");
    await renderWithProviders(page, { backend, route: `/runs/${id}` });
    expect(await screen.findByText("This run does not exist")).toBeInTheDocument();
    expect(job).not.toHaveBeenCalled();
    expect(log).not.toHaveBeenCalled();
  },
);

test("a run that does not exist has no log asked for either", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const log = vi.spyOn(backend, "jobLog");
  await renderWithProviders(page, { backend, route: "/runs/999" });
  expect(await screen.findByText("This run does not exist")).toBeInTheDocument();
  expect(log).not.toHaveBeenCalled();
});

test("trying again after the run could not be read reads it again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.job.bind(backend);
  backend.job = vi.fn().mockRejectedValueOnce(new BackendError("cannot read the job records")).mockImplementation(read);
  await renderWithProviders(page, { backend, route: "/runs/12" });
  expect(await screen.findByText("Could not read the run")).toBeInTheDocument();
  expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Runs");
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("heading", { level: 1, name: /DEMO-12/ })).toBeInTheDocument();
  expect(screen.queryByText("Could not read the run")).not.toBeInTheDocument();
});

test("another run opens on the timeline, not on the tab the last one was left on", async () => {
  const { client, backend } = await renderWithProviders(
    <>
      <Link to="/runs/10">next run</Link>
      {page}
    </>,
    { route: "/runs/12" },
  );
  // The next run is read already, so that the page has nothing to wait for between the two.
  await client.prefetchQuery({ queryKey: queryKeys.job(10), queryFn: () => backend.job(10) });
  await screen.findByRole("heading", { level: 1, name: /DEMO-12/ });
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  await userEvent.click(screen.getByRole("link", { name: "next run" }));
  expect(await screen.findByRole("heading", { level: 1, name: /DEMO-10/ })).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Timeline" })).toHaveAttribute("aria-selected", "true");
});

test("the page is in the language of the interface", async () => {
  await renderWithProviders(page, { route: "/runs/8", lang: "zh-CN" });
  expect(await screen.findByText("这次运行失败了")).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "时间线" })).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "原始日志" })).toBeInTheDocument();
  expect(field("状态")).toBe("失败");
  expect(field("时长")).toBe("1 小时 0 分");
});

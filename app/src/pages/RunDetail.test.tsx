import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Link, Route, Routes } from "react-router";
import { vi } from "vitest";
import { queryKeys } from "@/hooks/queries";
import type { Lang } from "@/i18n";
import { type Backend, BackendError } from "@/lib/backend";
import { MOCK_NOW, setFixedClock } from "@/lib/clock";
import { formatDateTime } from "@/lib/format";
import { createMockBackend } from "@/lib/mock";
import { ago } from "@/lib/mock-fixtures";
import type { Job, PlanProgress } from "@/lib/types";
import type { AcceptanceLinks, DispatchPreview, Handoff } from "@/lib/workflow-types";
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
async function openRun(id: number, backend: Backend = createMockBackend("demo", { latencyMs: 0 }), lang?: Lang) {
  const view = await renderWithProviders(page, { backend, route: `/runs/${id}`, lang });
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

const handoffFixture: Handoff = {
  job_id: 8,
  issue_identifier: "DEMO-8",
  executor: "claude",
  active: false,
  claimed_by: null,
  claim_id: null,
  prompt: "Continue from the saved index migration checkpoint.",
  review_prompt: "Independently review the index migration and its verification.",
  context: {
    revision: 1,
    origin_executor: "codex",
    preferred_executor: "claude",
    request: "Speed up the search index",
    scope: "Change the demo repository only",
    acceptance: ["The index returns the same results"],
    source_refs: ["Wiki: Demo Index", "Local: search-index.md"],
  },
  memory: {
    decision: { executor: "claude", source: "explicit" },
    checkpoint: {
      summary: "The index migration is in progress",
      done: ["Added the migration"],
      next: ["Verify the query plan"],
      verification: ["Unit tests passed"],
      blockers: ["The integration test remains"],
    },
    interruption: { code: "usage_limit_reached", reset_at: "2026-10-12T12:00:00Z" },
    git: { head: "0123456789abcdef0123456789abcdef01234567", branch: "demo/search-index", changed: ["src/index.ts"] },
    updated_at: "2026-10-12T09:00:00Z",
  },
};

test("continuing shows the selected strategy and only dispatches the reviewed executor's token", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.getContinuationPreview;
  backend.getContinuationPreview = vi.fn(async (...args: Parameters<typeof read>) => ({
    ...(await read(...args)), context: handoffFixture.context, wiki_root: "/demo/wiki",
    evidence: "Added the migration\nUnit tests passed\nVerify the query plan",
  }));
  let releaseApproval!: () => void;
  const pendingApproval = new Promise<void>((resolve) => { releaseApproval = resolve; });
  const queue = backend.approveContinuation;
  const approve = vi.spyOn(backend, "approveContinuation").mockImplementation(async (id, token) => {
    await pendingApproval;
    return queue(id, token);
  });
  const { client } = await openRun(8, backend);
  expect(screen.getByRole("button", { name: "Run again" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  const confirm = dialog.getByRole("button", { name: "Confirm and continue" });
  expect(confirm).toBeDisabled();
  expect(backend.getContinuationPreview).not.toHaveBeenCalled();
  await userEvent.click(dialog.getByRole("radio", { name: "Claude Code" }));
  expect(await dialog.findByText("Resume the original executor session")).toBeInTheDocument();
  await userEvent.click(dialog.getByRole("radio", { name: "Codex" }));
  expect(await dialog.findByText("Start a new session with the reviewed handoff")).toBeInTheDocument();
  expect(dialog.getByText("Wiki: Demo Index")).toBeInTheDocument();
  expect(dialog.getByText(/Added the migration.*Unit tests passed.*Verify the query plan/)).toBeInTheDocument();
  expect(approve).not.toHaveBeenCalled();
  await userEvent.click(confirm);
  await act(async () => {
    releaseApproval();
    await approve.mock.results[0].value;
  });
  expect(await screen.findByRole("link", { name: "Continued from run #8" })).toBeInTheDocument();
  await waitFor(() => expect(client.isFetching()).toBe(0));
  expect(approve).toHaveBeenCalledExactlyOnceWith(8, "demo-continuation-2");
  expect(await backend.job(13)).toMatchObject({ executor: "codex", state: "queued" });
  expect(await backend.job(8)).toMatchObject({ executor: "claude", state: "failed" });
});

test("a needs-information implementation shows what to supplement and allows either executor", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(8))!;
  const needsInfo: Job = { ...source, mode: "implement", state: "needs_info", report: "needs_info" };
  const readJob = backend.job;
  backend.job = vi.fn((id) => id === source.id ? Promise.resolve(needsInfo) : readJob(id));
  backend.getHandoff = vi.fn().mockResolvedValue(handoffFixture);
  backend.getContinuationContext = vi.fn().mockResolvedValue(handoffFixture.context);
  const read = backend.getContinuationPreview;
  backend.getContinuationPreview = vi.fn(async (...args: Parameters<typeof read>) => ({
    ...(await read(...args)),
    context: { ...handoffFixture.context!, conversation_history: "The demo fixture is on the second page." },
  }));
  const approve = vi.spyOn(backend, "approveContinuation");
  await openRun(source.id, backend);
  const requested = within(await screen.findByRole("region", { name: "Information requested" }));
  expect(requested.getByText("The integration test remains")).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  const dialogRequest = await dialog.findByRole("region", { name: "Information requested" });
  expect(within(dialogRequest).getByText("The integration test remains")).toBeInTheDocument();
  const supplement = await dialog.findByRole("textbox", { name: "Additional task information" });
  expect(dialogRequest.compareDocumentPosition(supplement) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(dialog.getByRole("radio", { name: "Codex" })).toBeInTheDocument();
  await userEvent.click(dialog.getByRole("radio", { name: "Claude Code" }));
  expect(await dialog.findByText("Resume the original executor session")).toBeInTheDocument();
  expect(dialog.getByText("The demo fixture is on the second page.")).toBeInTheDocument();
  expect(dialog.getByText("Wiki: Demo Index")).toBeInTheDocument();
  expect(approve).not.toHaveBeenCalled();
  await userEvent.click(dialog.getByRole("button", { name: "Confirm and continue" }));
  expect(await screen.findByRole("link", { name: "Continued from run #8" })).toBeInTheDocument();
  expect(approve).toHaveBeenCalledExactlyOnceWith(source.id, "demo-continuation-1");
});

test("supplementing a needs-information run preserves its context and requires a new preview", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(8))!;
  const readJob = backend.job;
  const needsInfo: Job = { ...source, state: "needs_info", mode: "implement", report: "needs_info" };
  backend.job = vi.fn((id) => id === source.id
    ? Promise.resolve(needsInfo)
    : readJob(id));
  let current = { ...handoffFixture.context!, conversation_history: "Original decision: preserve the index." };
  backend.getContinuationContext = vi.fn(async () => current);
  const preview = backend.getContinuationPreview;
  backend.getContinuationPreview = vi.fn(async (...args: Parameters<typeof preview>) => ({
    ...(await preview(...args)), context: current,
  }));
  let releaseSave!: () => void;
  const pendingSave = new Promise<void>((resolve) => { releaseSave = resolve; });
  const save = vi.spyOn(backend, "supplementContinuationContext").mockImplementation(async (_id, revision, text) => {
    await pendingSave;
    current = { ...current, revision: revision + 1, conversation_history: `${current.conversation_history}\n\n${text}` };
    return current;
  });
  const approve = vi.spyOn(backend, "approveContinuation");
  const { client } = await openRun(source.id, backend);
  await userEvent.click(await screen.findByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  const information = await dialog.findByRole("textbox", { name: "Additional task information" });
  expect(information).toHaveValue("");
  const saveButton = dialog.getByRole("button", { name: "Save information and preview" });
  expect(saveButton).toBeDisabled();
  await userEvent.click(dialog.getByRole("radio", { name: "Codex" }));
  await dialog.findByText("Start a new session with the reviewed handoff");
  const confirm = dialog.getByRole("button", { name: "Confirm and continue" });
  await waitFor(() => expect(confirm).toBeEnabled());

  await userEvent.type(information, "The integration fixture is on the second page.");
  expect(confirm).toBeDisabled();
  await userEvent.click(saveButton);
  expect(save).toHaveBeenCalledExactlyOnceWith(source.id, 1, "The integration fixture is on the second page.");
  expect(confirm).toBeDisabled();
  expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
  expect(dialog.getByRole("radio", { name: "Claude Code" })).toBeDisabled();
  await act(async () => { releaseSave(); await save.mock.results[0].value; });
  await waitFor(() => expect(client.isFetching()).toBe(0));
  expect(information).toHaveValue("");
  expect(dialog.getByText(/Original decision: preserve the index.*The integration fixture is on the second page\./)).toBeInTheDocument();
  expect(dialog.getByText("Wiki: Demo Index")).toBeInTheDocument();
  expect(dialog.getByText("Change the demo repository only")).toBeInTheDocument();
  expect(confirm).toBeEnabled();
  expect(approve).not.toHaveBeenCalled();
  expect(await backend.job(source.id)).toMatchObject({ state: "needs_info" });
});

test("failed supplemental information stays editable and cannot be confirmed as sent", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({ ...source, state: "needs_info", mode: "implement", report: "needs_info" });
  backend.getContinuationContext = vi.fn().mockResolvedValue(handoffFixture.context);
  backend.supplementContinuationContext = vi.fn().mockRejectedValue(new BackendError("issue context revision changed; read it and try again"));
  const approve = vi.spyOn(backend, "approveContinuation");
  const { client } = await openRun(source.id, backend);
  await userEvent.click(await screen.findByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  const information = await dialog.findByRole("textbox", { name: "Additional task information" });
  await userEvent.click(dialog.getByRole("radio", { name: "Codex" }));
  await userEvent.type(information, "The fixture is on the second page.");
  await userEvent.click(dialog.getByRole("button", { name: "Save information and preview" }));
  expect(await dialog.findByText("Could not update task information")).toBeInTheDocument();
  expect(information).toHaveValue("The fixture is on the second page.");
  expect(dialog.getByRole("button", { name: "Confirm and continue" })).toBeDisabled();
  expect(approve).not.toHaveBeenCalled();
  await waitFor(() => expect(client.isFetching()).toBe(0));
});

test("a needs-information run waits for its current context and can retry a failed read", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({ ...source, state: "needs_info", mode: "implement", report: "needs_info" });
  backend.getContinuationContext = vi.fn()
    .mockRejectedValueOnce(new BackendError("cannot read the current task brief"))
    .mockResolvedValue(handoffFixture.context);
  await openRun(source.id, backend);
  await userEvent.click(await screen.findByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  expect(await dialog.findByText("cannot read the current task brief")).toBeInTheDocument();
  expect(dialog.queryByRole("textbox", { name: "Additional task information" })).not.toBeInTheDocument();
  expect(dialog.getByRole("button", { name: "Confirm and continue" })).toBeDisabled();
  await userEvent.click(dialog.getByRole("button", { name: "Try again" }));
  expect(await dialog.findByRole("textbox", { name: "Additional task information" })).toHaveValue("");
});

test("a needs-information implementation without recorded blockers points to the latest issue comment", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(8))!;
  const needsInfo: Job = { ...source, mode: "implement", state: "needs_info", report: "needs_info" };
  backend.job = vi.fn().mockResolvedValue(needsInfo);
  backend.getHandoff = vi.fn().mockResolvedValue({ ...handoffFixture, memory: null });
  await openRun(source.id, backend);
  const requested = within(await screen.findByRole("region", { name: "Information requested" }));
  expect(requested.getByText(/latest Linear comment/)).toBeInTheDocument();
  await userEvent.click(await screen.findByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  const dialogRequest = within(await dialog.findByRole("region", { name: "Information requested" }));
  expect(dialogRequest.getByText(/latest Linear comment/)).toBeInTheDocument();
});

test("the continuation dialog retries a failed read of the information requested", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({ ...source, mode: "implement", state: "needs_info", report: "needs_info" });
  backend.getHandoff = vi.fn().mockRejectedValue(new BackendError("cannot read the saved checkpoint"));
  const approve = vi.spyOn(backend, "approveContinuation");
  const save = vi.spyOn(backend, "supplementContinuationContext");
  await openRun(source.id, backend);
  await userEvent.click(await screen.findByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  expect(await dialog.findByText("Could not read the handoff")).toBeInTheDocument();
  expect(dialog.getByText("cannot read the saved checkpoint")).toBeInTheDocument();

  backend.getHandoff = vi.fn().mockResolvedValue(handoffFixture);
  await userEvent.click(dialog.getByRole("button", { name: "Try again" }));
  const requested = within(await dialog.findByRole("region", { name: "Information requested" }));
  expect(requested.getByText("The integration test remains")).toBeInTheDocument();
  expect(dialog.queryByText("Could not read the handoff")).not.toBeInTheDocument();
  expect(approve).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
});

test("recording local completion requires confirmation, refreshes the run, and keeps its checkpoint", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const original = (await backend.job(9))!;
  backend.getHandoff = vi.fn().mockResolvedValue({ ...handoffFixture, job_id: original.id });
  const record = vi.spyOn(backend, "recordLocalCompletion");
  const markDone = vi.spyOn(backend, "markDone");

  await openRun(original.id, backend);
  const action = await screen.findByRole("button", { name: "Record local completion" });
  await userEvent.click(action);
  let dialog = within(screen.getByRole("alertdialog"));
  expect(dialog.getByText(/development and verification.*Linear issue remains unchanged/)).toBeInTheDocument();
  expect(record).not.toHaveBeenCalled();
  await userEvent.click(dialog.getByRole("button", { name: "Cancel" }));
  expect(record).not.toHaveBeenCalled();

  await userEvent.click(action);
  dialog = within(screen.getByRole("alertdialog"));
  await userEvent.click(dialog.getByRole("button", { name: "Confirm" }));
  expect(record).toHaveBeenCalledExactlyOnceWith(original.id);
  expect(markDone).not.toHaveBeenCalled();
  expect(await screen.findByText("Awaiting acceptance")).toBeInTheDocument();
  expect(screen.getByText("Development and verification are complete on this computer.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Mark done" })).not.toBeInTheDocument();
  expect(screen.queryByText("Needs information")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Run again" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Record local completion" })).not.toBeInTheDocument();
  expect(screen.getByText("The integration test remains")).toBeInTheDocument();
  expect(await backend.job(original.id)).toMatchObject({ state: "local_complete", report: "needs_info" });
});

test.each([
  { mode: "plan", report: "planned" },
  { mode: "implement", finished_at: null },
  { mode: "implement", started_at: null },
  { mode: "implement", report: null },
  { mode: "implement", state: "done" },
] as const)("an ineligible run offers no local completion action: %j", async (patch) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(9))!;
  const job: Job = { ...source, ...patch };
  backend.job = vi.fn().mockResolvedValue(job);
  await openRun(job.id, backend);
  expect(screen.queryByRole("button", { name: "Record local completion" })).not.toBeInTheDocument();
});

test.each(["superseded", "claimed"] as const)("a %s run offers no local completion action", async (obstacle) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(9))!;
  if (obstacle === "superseded") {
    const list = backend.listJobs;
    backend.listJobs = vi.fn(async (limit) => [{ ...source, id: 13 }, ...await list(limit)]);
  } else {
    backend.getHandoff = vi.fn().mockResolvedValue({ ...handoffFixture, job_id: source.id, claimed_by: "codex" });
  }
  const { client } = await openRun(source.id, backend);
  await waitFor(() => expect(client.isFetching()).toBe(0));
  expect(screen.queryByRole("button", { name: "Record local completion" })).not.toBeInTheDocument();
});

test.each([8, 13])("run #%i can use its unstarted follow-up without dispatching on cancel", async (id) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(8))!;
  const prepared: Job = {
    ...source, id: 13, state: "waiting_binding", resume_job_id: source.id,
    started_at: null, finished_at: null, reason: "waiting for dispatch review",
  };
  const readJob = backend.job;
  backend.job = vi.fn((id) => id === prepared.id ? Promise.resolve(prepared) : readJob(id));
  const listJobs = backend.listJobs;
  backend.listJobs = vi.fn(async (limit) => [prepared, ...await listJobs(limit)]);
  const preview = vi.spyOn(backend, "getContinuationPreview");
  const approve = vi.spyOn(backend, "approveContinuation");
  await openRun(id, backend);
  await userEvent.click(await screen.findByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  await userEvent.click(dialog.getByRole("radio", { name: "Codex" }));
  expect(await dialog.findByText("Start a new session with the reviewed handoff")).toBeInTheDocument();
  expect(preview).toHaveBeenCalledExactlyOnceWith(source.id, "codex");
  await userEvent.click(dialog.getByRole("button", { name: "Cancel" }));
  expect(approve).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: "Preview" })).not.toBeInTheDocument();
});

test("a rejected continuation approval keeps the preview open and requires a new preview", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const approve = vi.spyOn(backend, "approveContinuation").mockRejectedValueOnce(new BackendError("preview changed"));
  await openRun(8, backend);
  await userEvent.click(screen.getByRole("button", { name: "Continue" }));
  const dialog = within(screen.getByRole("dialog"));
  await userEvent.click(dialog.getByRole("radio", { name: "Codex" }));
  const confirm = dialog.getByRole("button", { name: "Confirm and continue" });
  expect(await dialog.findByText("Start a new session with the reviewed handoff")).toBeInTheDocument();
  await userEvent.click(confirm);
  expect(await dialog.findByText("preview changed")).toBeInTheDocument();
  expect(confirm).toBeDisabled();
  await userEvent.click(dialog.getByRole("button", { name: "Reload preview" }));
  await waitFor(() => expect(confirm).toBeEnabled());
  await userEvent.click(confirm);
  expect(await screen.findByRole("link", { name: "Continued from run #8" })).toBeInTheDocument();
  expect(approve.mock.calls).toEqual([[8, "demo-continuation-1"], [8, "demo-continuation-2"]]);
});

test("a finished Claude session can be reopened in its own client using only the recorded ID", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const session = (await backend.job(8))!.session_id;
  const read = vi.spyOn(backend, "readCodexSession");
  const copy = stubClipboard();
  await openRun(8, backend);
  const card = within(screen.getByRole("region", { name: "Original executor session" }));
  await userEvent.click(card.getByRole("button", { name: "Copy reopen command" }));
  expect(copy).toHaveBeenCalledExactlyOnceWith(`claude --resume ${session}`);
  await userEvent.click(card.getByRole("button", { name: "Copy Claude Desktop command" }));
  expect(copy).toHaveBeenLastCalledWith(`claude --desktop --resume ${session}`);
  expect(card.queryByText(/Wait for this run to end/)).not.toBeInTheDocument();
  expect(card.queryByRole("button", { name: "Preview Codex session" })).not.toBeInTheDocument();
  expect(read).not.toHaveBeenCalled();
});

test("a running Codex session shows a copyable Desktop link without opening or reading it", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({
    ...job, executor: "codex", state: "running", finished_at: null,
  });
  const read = vi.spyOn(backend, "readCodexSession");
  const copy = stubClipboard();
  await openRun(job.id, backend);
  const card = within(screen.getByRole("region", { name: "Original executor session" }));
  const link = `codex://threads/${job.session_id}`;
  expect(card.getByText(link, { selector: "code" })).toBeInTheDocument();
  await userEvent.click(card.getByRole("button", { name: "Copy Codex Desktop link" }));
  expect(copy).toHaveBeenCalledExactlyOnceWith(link);
  expect(card.queryByRole("link")).not.toBeInTheDocument();
  expect(card.queryByRole("button", { name: "Copy reopen command" })).not.toBeInTheDocument();
  expect(card.queryByRole("button", { name: "Preview Codex session" })).not.toBeInTheDocument();
  expect(read).not.toHaveBeenCalled();
});

test("a running Claude session shows the Desktop command with a wait-for-end hint", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({ ...job, state: "running", finished_at: null });
  const copy = stubClipboard();
  await openRun(job.id, backend);
  const card = within(screen.getByRole("region", { name: "Original executor session" }));
  const command = `claude --desktop --resume ${job.session_id}`;
  expect(card.getByText(command, { selector: "code" })).toBeInTheDocument();
  expect(card.getByText(/Wait for this run to end/)).toBeInTheDocument();
  await userEvent.click(card.getByRole("button", { name: "Copy Claude Desktop command" }));
  expect(copy).toHaveBeenCalledExactlyOnceWith(command);
  expect(card.queryByRole("button", { name: "Copy reopen command" })).not.toBeInTheDocument();
});

test.each([
  ["waiting for a binding", { state: "waiting_binding" }],
  ["queued", { state: "queued" }],
  ["missing the start time", { state: "running", started_at: null }],
  ["missing the working folder", { state: "running", directory: null }],
  ["missing the executor", { state: "running", executor: null }],
  ["missing the session ID", { state: "running", session_id: null }],
] satisfies [string, Partial<Job>][])("a run %s has no provider session actions", async (_label, fields) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({ ...job, finished_at: null, ...fields });
  await openRun(job.id, backend);
  expect(screen.queryByRole("region", { name: "Original executor session" })).not.toBeInTheDocument();
});

test("Codex session preview only reads after a click, and displays text from the recorded session", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.job(8))!;
  const readJob = backend.job;
  backend.job = vi.fn((id) => id === job.id ? Promise.resolve({ ...job, executor: "codex" as const }) : readJob(id));
  backend.readCodexSession = vi.fn().mockResolvedValue({
    session_id: job.session_id,
    title: "Index work",
    messages: [{ role: "user", text: "<b>Review changes</b>" }, { role: "assistant", text: "Focused tests passed." }],
    has_more: true,
  });
  const copy = stubClipboard();
  await openRun(8, backend);
  const card = within(screen.getByRole("region", { name: "Original executor session" }));
  expect(backend.readCodexSession).not.toHaveBeenCalled();
  await userEvent.click(card.getByRole("button", { name: "Copy reopen command" }));
  expect(copy).toHaveBeenCalledExactlyOnceWith(`codex resume ${job.session_id}`);
  await userEvent.click(card.getByRole("button", { name: "Copy Codex Desktop link" }));
  expect(copy).toHaveBeenLastCalledWith(`codex://threads/${job.session_id}`);
  expect(backend.readCodexSession).not.toHaveBeenCalled();
  await userEvent.click(card.getByRole("button", { name: "Preview Codex session" }));
  const dialog = within(screen.getByRole("dialog", { name: "Preview Codex session" }));
  expect(await dialog.findByText("<b>Review changes</b>")).toBeInTheDocument();
  expect(dialog.getByText("Focused tests passed.")).toBeInTheDocument();
  expect(dialog.getByText("Open the original Codex session to read earlier messages.")).toBeInTheDocument();
  expect(backend.readCodexSession).toHaveBeenCalledExactlyOnceWith(8);
});

test("a run with a malformed session ID has no provider session actions", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.job(8))!;
  const readJob = backend.job;
  backend.job = vi.fn((id) => id === job.id ? Promise.resolve({ ...job, session_id: `${job.session_id}\n` }) : readJob(id));
  await openRun(8, backend);
  expect(screen.queryByRole("region", { name: "Original executor session" })).not.toBeInTheDocument();
});

test("a run handoff explains the saved context and copies the independent review prompt", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.getHandoff = vi.fn().mockResolvedValue(handoffFixture);
  const copy = stubClipboard();
  await openRun(8, backend);
  const handoff = within(await screen.findByRole("region", { name: "Run handoff" }));
  expect(await handoff.findByText("Quota exhausted")).toBeInTheDocument();
  expect(handoff.getByText(formatDateTime("2026-10-12T12:00:00Z", "en"))).toBeInTheDocument();
  expect(handoff.getByText("Claude Code · Explicit choice")).toBeInTheDocument();
  expect(handoff.getByText("Verify the query plan")).toBeInTheDocument();
  expect(handoff.getByText("Wiki: Demo Index")).toBeInTheDocument();
  expect(handoff.getByText("src/index.ts")).toBeInTheDocument();
  await userEvent.click(handoff.getByRole("button", { name: "Copy independent review prompt" }));
  expect(copy).toHaveBeenCalledWith("Independently review the index migration and its verification.");
});

test.each([false, true])("the Wiki archive card shows the recorded receipt when available (%s)", async (recorded) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.getHandoff = vi.fn().mockResolvedValue({
    ...handoffFixture,
    archive: {
      root: "/demo/wiki",
      prompt: "Archive the index migration in the configured Wiki.",
      receipt: recorded ? {
        page: "projects/index-migration.md",
        commit: "0123456789abcdef0123456789abcdef01234567",
        recorded_at: "2026-10-12T13:00:00Z",
      } : null,
    },
  });
  const copy = stubClipboard();
  await openRun(8, backend);
  const archive = within(await screen.findByRole("region", { name: "Wiki archive" }));
  expect(archive.getByText("/demo/wiki")).toBeInTheDocument();
  if (recorded) {
    expect(archive.getByText("projects/index-migration.md")).toBeInTheDocument();
    expect(archive.getByText("0123456789abcdef0123456789abcdef01234567")).toBeInTheDocument();
  } else {
    expect(archive.getByText("No archive receipt recorded yet.")).toBeInTheDocument();
  }
  await userEvent.click(archive.getByRole("button", { name: "Copy Wiki archive material" }));
  expect(copy).toHaveBeenCalledWith("Archive the index migration in the configured Wiki.");
});

test("independent verification only queues after the exact delivery preview is confirmed", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const prepareAction = backend.prepareControlAction;
  const prepare = vi.spyOn(backend, "prepareControlAction").mockImplementation(async (action) => {
    const preview = await prepareAction(action);
    return {
      ...preview,
      acceptance: {
        ...preview.acceptance!,
        scope: "Verify the saved delivery in the sandbox.",
        brief_revision: 4,
        context: {
          revision: 4, origin_executor: "codex", preferred_executor: "codex",
          request: "Verify the revised two-tab save behavior.",
          conversation_history: "Check both the retry and the original saved record.",
          scope: "Previous draft scope",
          acceptance: ["Both tabs keep the same saved record.", "A rejected save leaves the draft intact."],
          source_refs: ["wiki/queries/revised-save-criteria.md"],
        },
      },
    };
  });
  const commit = vi.spyOn(backend, "commitControlAction");
  await openRun(10, backend);
  await userEvent.click(await screen.findByRole("button", { name: "Start independent verification" }));
  const dialog = within(screen.getByRole("dialog", { name: "Start independent verification" }));
  const delivery = dialog.getByRole("textbox", { name: "Delivery to verify" });
  const confirm = dialog.getByRole("button", { name: "Confirm action" });
  expect(confirm).toBeDisabled();
  await userEvent.click(dialog.getByRole("radio", { name: "Codex" }));
  await userEvent.type(delivery, "build-demo-3");
  await userEvent.click(dialog.getByRole("button", { name: "Review action" }));
  expect(await dialog.findByText("Verification sandbox with network access")).toBeInTheDocument();
  expect(dialog.getByText(/Network access is enabled; the executor must follow this task's authorized environment scope/)).toBeInTheDocument();
  expect(dialog.getByText("build-demo-3", { selector: "dd" })).toBeInTheDocument();
  const fingerprint = "0123456789abcdef".repeat(4);
  expect(dialog.getByText(fingerprint, { selector: "code" })).toHaveAttribute("title", fingerprint);
  const source = (await backend.job(10))!;
  const revisedAt = source.verified_updated_at ?? source.discovered_updated_at;
  expect(dialog.getByText(formatDateTime(revisedAt, "en"), { selector: "time" })).toHaveAttribute("dateTime", revisedAt);
  expect(dialog.getByText("Task brief revision", { selector: "dt" }).nextElementSibling).toHaveTextContent("4");
  expect(dialog.getByText("Verify the revised two-tab save behavior.")).toBeInTheDocument();
  expect(dialog.getByText("Check both the retry and the original saved record.")).toBeInTheDocument();
  expect(dialog.getByText("Verify the saved delivery in the sandbox.")).toBeInTheDocument();
  expect(dialog.queryByText("Previous draft scope")).not.toBeInTheDocument();
  expect(dialog.getByText("Both tabs keep the same saved record.", { selector: "li" })).toBeInTheDocument();
  expect(dialog.getByText("A rejected save leaves the draft intact.", { selector: "li" })).toBeInTheDocument();
  expect(dialog.getByText("wiki/queries/revised-save-criteria.md", { selector: "li" })).toBeInTheDocument();
  expect(prepare).toHaveBeenCalledWith({ action: "start_verification", job_id: 10, executor: "codex", delivery: "build-demo-3" });
  expect(commit).not.toHaveBeenCalled();

  await userEvent.clear(delivery);
  await userEvent.type(delivery, "build-demo-4");
  expect(confirm).toBeDisabled();
  await userEvent.click(dialog.getByRole("button", { name: "Review action" }));
  expect(await dialog.findByText("build-demo-4", { selector: "dd" })).toBeInTheDocument();
  await userEvent.click(confirm);
  expect(commit).toHaveBeenCalledExactlyOnceWith("demo-acceptance-2");
  expect(await screen.findByText("Verification is queued")).toBeInTheDocument();
  expect(await screen.findByRole("link", { name: "Development run #10" })).toHaveAttribute("href", "/runs/10");
  expect(await backend.job(13)).toMatchObject({ mode: "verify", state: "queued", resume_job_id: null });
  expect(screen.queryByRole("button", { name: "Mark done" })).not.toBeInTheDocument();
});

test("a verification blocks development closure until it passes", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.getHandoff;
  let acceptance: AcceptanceLinks = {
    development_job_id: 10, repair_of_job_id: null, verification_job_id: 13, delivery: "build-demo-3",
    verification_status: "failed", returned_for_repair: false, repair_reason: null,
  };
  backend.getHandoff = async (id) => ({ ...(await read(id)), acceptance });
  const { client } = await openRun(10, backend);
  expect(await screen.findByRole("link", { name: "Verification run #13" })).toHaveAttribute("href", "/runs/13");
  expect(screen.getByText("Verification did not pass")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Mark done" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Start independent verification" })).not.toBeInTheDocument();
  acceptance = { ...acceptance, verification_status: "passed" };
  await act(() => client.invalidateQueries({ queryKey: queryKeys.handoff(10) }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Mark done" })).toBeEnabled());
});

test("a failed independent verification can return for repair with reviewed findings", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const prepared = await backend.prepareControlAction({
    action: "start_verification", job_id: 10, executor: "codex", delivery: "build-demo-3",
  });
  await backend.commitControlAction(prepared.token);
  let saved: AcceptanceLinks = { ...(await backend.getHandoff(13)).acceptance!, verification_status: "failed" };
  const readHandoff = backend.getHandoff;
  backend.getHandoff = async (id) => ({ ...(await readHandoff(id)), acceptance: saved });
  const read = backend.job;
  backend.job = async (id) => id === 13 ? {
    ...(await read(id))!, state: "failed", report: "verification_failed", started_at: ago(3), finished_at: ago(1),
  } : read(id);
  const repair = { action: "return_for_repair", job_id: 13, reason: "Repeat the save with two tabs." } as const;
  const prepare = vi.fn<Backend["prepareControlAction"]>().mockResolvedValue({
    token: "demo-repair-preview", action: repair, issue_identifier: "DEMO-10", job_id: 13, current_state: "failed",
    acceptance: {
      source_job_id: 10, delivery: "build-demo-3", reason: repair.reason,
      executor: null, directory: null, model: null, effort: null, permission: null, lean: null,
    },
    expires_in_seconds: 180,
  });
  const commit = vi.fn<Backend["commitControlAction"]>(async () => {
    saved = { ...saved, returned_for_repair: true, repair_reason: repair.reason };
    return { action: repair, issue_identifier: "DEMO-10", job_id: 13, result: "returned_for_repair" };
  });
  backend.prepareControlAction = prepare;
  backend.commitControlAction = commit;
  await openRun(13, backend);
  expect(await screen.findByText("Verification did not pass")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Mark done" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Run again" })).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Return for repair" }));
  const dialog = within(screen.getByRole("dialog", { name: "Return for repair" }));
  await userEvent.type(dialog.getByRole("textbox", { name: "Repair findings and reproduction" }), "Repeat the save with two tabs.");
  await userEvent.click(dialog.getByRole("button", { name: "Review action" }));
  expect(await dialog.findByText("Repeat the save with two tabs.", { selector: "dd" })).toBeInTheDocument();
  expect(dialog.queryByText("Delivery fingerprint")).not.toBeInTheDocument();
  expect(prepare).toHaveBeenCalledExactlyOnceWith(repair);
  expect(commit).not.toHaveBeenCalled();
  await userEvent.click(dialog.getByRole("button", { name: "Confirm action" }));
  expect(commit).toHaveBeenCalledExactlyOnceWith("demo-repair-preview");
  expect(await screen.findByText("Returned for repair")).toBeInTheDocument();
  expect(screen.getByText(/create a new revision/)).toBeInTheDocument();
  expect((await backend.getHandoff(10)).acceptance).toMatchObject({ returned_for_repair: true, repair_reason: "Repeat the save with two tabs." });
});

test("a finished run can record a Wiki receipt and reads the saved page and commit back", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await backend.saveSettings({ ...(await backend.settings()), wiki_root: "/demo/wiki" });
  const record = vi.spyOn(backend, "recordArchive");
  await openRun(8, backend);
  const archive = within(await screen.findByRole("region", { name: "Wiki archive" }));
  await userEvent.click(archive.getByRole("button", { name: "Record archive receipt" }));
  const dialog = within(screen.getByRole("dialog", { name: "Record archive receipt" }));
  const commit = "0123456789abcdef0123456789abcdef01234567";
  await userEvent.type(dialog.getByRole("textbox", { name: "Archived page" }), "projects/demo.md");
  await userEvent.type(dialog.getByRole("textbox", { name: "Archive commit" }), commit);
  expect(record).not.toHaveBeenCalled();
  await userEvent.click(dialog.getByRole("button", { name: "Save receipt" }));
  expect(record).toHaveBeenCalledExactlyOnceWith(8, "projects/demo.md", commit);
  expect(await archive.findByText("projects/demo.md")).toBeInTheDocument();
  expect(archive.getByText(commit)).toBeInTheDocument();
});

test("claiming a handoff requires choosing an executor and leaves the run's executor intact", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  // The other demo run uses the same folder. End it so this handoff can reserve the folder.
  await backend.cancelJob(12);
  const claimHandoff = vi.spyOn(backend, "claimHandoff");
  const releaseHandoff = vi.spyOn(backend, "releaseHandoff");
  await openRun(8, backend);
  const card = within(await screen.findByRole("region", { name: "Run handoff" }));
  const claim = await card.findByRole("button", { name: "Claim handoff" });
  expect(claim).toBeDisabled();
  await userEvent.click(card.getByRole("combobox", { name: "Takeover executor" }));
  await userEvent.click(await screen.findByRole("option", { name: "Codex" }));
  await userEvent.click(claim);
  expect(await card.findByText("Claimed for Codex")).toBeInTheDocument();
  expect(claimHandoff).toHaveBeenCalledWith(8, "codex");
  expect(field("Executor")).toBe("Claude Code");
  expect(field("Status")).toBe("Failed");
  const claimId = (await backend.getHandoff(8)).claim_id;
  expect(claimId).toEqual(expect.any(String));
  await userEvent.click(card.getByRole("button", { name: "Release handoff" }));
  expect(await card.findByRole("button", { name: "Claim handoff" })).toBeDisabled();
  expect(releaseHandoff).toHaveBeenCalledWith(8, claimId);
});

test.each([
  [12, true, "Wait for this run to finish before claiming its handoff."],
  [7, false, "This run is done."],
] as const)("run %s cannot be claimed", async (id, active, message) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.getHandoff = vi.fn().mockResolvedValue({ ...handoffFixture, job_id: id, active });
  await openRun(id, backend);
  const handoff = within(await screen.findByRole("region", { name: "Run handoff" }));
  expect(await handoff.findByText(message)).toBeInTheDocument();
  expect(handoff.queryByRole("button", { name: "Claim handoff" })).not.toBeInTheDocument();
});

test("a failed handoff read can be retried independently of the run", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.getHandoff = vi.fn()
    .mockRejectedValueOnce(new BackendError("handoff store unavailable"))
    .mockResolvedValue(handoffFixture);
  await openRun(8, backend);
  const handoff = within(await screen.findByRole("region", { name: "Run handoff" }));
  expect(await handoff.findByText("Could not read the handoff")).toBeInTheDocument();
  expect(handoff.getByText("handoff store unavailable")).toBeInTheDocument();
  expect(field("Executor")).toBe("Claude Code");
  await userEvent.click(handoff.getByRole("button", { name: "Try again" }));
  expect(await handoff.findByText("Verify the query plan")).toBeInTheDocument();
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

test("a Claude run's usage is its estimated cost, its tokens, its cache and its turns, with what the cost is", async () => {
  await openRun(10);
  expect(field("Usage")).toBe(
    "$0.42 (estimate) · 1,204 tokens in, 3,872 out · cache 186,402 read, 21,390 written · 17 turns" +
      "The cost is Claude Code's own estimate at API prices. When Claude Code is signed in with a subscription, it is not what you are charged.",
  );
});

test("a Codex run's usage is its tokens, with a note that its input counts the cache", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const done = (await backend.job(7))!;
  const usage = {
    cost_micro_usd: null,
    session_cost_micro_usd: null,
    input_tokens: 45_290,
    output_tokens: 18,
    cache_read_tokens: 39_040,
    cache_write_tokens: null,
    turns: null,
  };
  backend.job = vi.fn().mockResolvedValue({ ...done, executor: "codex", usage });
  await openRun(7, backend, "zh-CN");
  expect(field("用量")).toBe("输入 45,290、输出 18 token · 缓存读取 39,040Codex 只报 token；它的输入数已包含缓存读取的部分。");
});

test("a run that reported no usage, or ran before usage was recorded, has no usage field", async () => {
  await openRun(7);
  expect(screen.queryByText("Usage", { selector: "dt" })).not.toBeInTheDocument();
});

test("a run that continues an earlier session links to that run", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const done = (await backend.job(7))!;
  const read = backend.job.bind(backend);
  backend.job = vi.fn(async (id: number) => (id === 7 ? { ...done, resume_job_id: 6 } : read(id)));
  await openRun(7, backend);
  const link = screen.getByRole("link", { name: "Continued from run #6" });
  expect(link).toHaveAttribute("href", "/runs/6");
  expect(screen.getByText("Continued from", { selector: "dt" })).toBeInTheDocument();
});

test("a planning run says what it was for, and one that posted its plan has its own badge", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const done = (await backend.job(9))!;
  backend.job = vi.fn().mockResolvedValue({ ...done, mode: "plan", report: "planned", summary: "Posted a plan in three steps" });
  await openRun(9, backend);
  expect(field("Status")).toBe("Plan posted");
  expect(field("Run for")).toBe("Plan only");
  expect(screen.getByRole("button", { name: "Implement" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument();
  expect(screen.getByText(/The plan is saved\..*Implementation applies only to DEMO-9\./)).toBeInTheDocument();
});

test.each([
  { state: "needs_info", report: "needs_info" },
  { state: "failed", report: "failed" },
  { state: "running", report: "planned", finished_at: null },
  { state: "needs_info", report: "planned", finished_at: null },
] as const)("a plan without a finished successful result offers no implementation action: %j", async (patch) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.job(9))!;
  backend.job = vi.fn().mockResolvedValue({ ...job, ...patch, mode: "plan" });
  await openRun(9, backend);
  expect(screen.queryByRole("button", { name: "Implement" })).not.toBeInTheDocument();
});

test("implementation opens the planned issue in Linear and waits for its exact implementation preview", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.job(9))!;
  backend.job = vi.fn().mockResolvedValue({ ...job, mode: "plan", report: "planned" });
  const preview: DispatchPreview = {
    token: "implement-preview",
    issue_uuid: job.issue_uuid,
    issue_identifier: job.issue_identifier,
    title: job.title,
    mode: "implement",
    executor: "codex",
    directory: "~/code/demo-app",
    model: "gpt-6.1-sol",
    effort: "ultra",
    wiki_root: "~/notes/demo-wiki",
    context: {
      revision: 1,
      origin_executor: "codex",
      preferred_executor: "codex",
      request: "Use the revised export format.",
      conversation_history: "The first format was rejected.\n\tThe final choice preserves the date in UTC.",
      scope: "Update the export format.",
      acceptance: ["Exports preserve the date in UTC."],
      source_refs: ["wiki/exports.md#format"],
    },
  };
  vi.spyOn(backend, "getDispatchPreview")
    .mockRejectedValueOnce(new BackendError("issue no longer matches the dispatch conditions"))
    .mockResolvedValueOnce({ ...preview, mode: "plan", token: "plan-preview" })
    .mockResolvedValueOnce({ ...preview, issue_uuid: "different-workspace-issue", token: "unrelated-preview" })
    .mockResolvedValue(preview);
  const issueUrl = `https://linear.app/demo/issue/${job.issue_identifier}`;
  const getUrl = vi.spyOn(backend, "getDevelopmentIssueUrl").mockResolvedValue(issueUrl);
  const openLinear = vi.spyOn(backend, "openLinear");
  const approve = vi.spyOn(backend, "approveDispatch").mockResolvedValue(undefined);
  await openRun(9, backend);
  expect(getUrl).not.toHaveBeenCalled();

  await userEvent.click(screen.getByRole("button", { name: "Implement" }));
  const review = within(screen.getByRole("dialog", { name: "Review dispatch" }));
  const confirm = review.getByRole("button", { name: "Confirm dispatch" });
  expect(await review.findByText(/set this issue to Todo, assign it to yourself, and add ai-ready/)).toBeInTheDocument();
  expect(confirm).toBeDisabled();
  expect(getUrl).not.toHaveBeenCalled();
  await userEvent.click(review.getByRole("button", { name: "Open in Linear" }));
  await waitFor(() => expect(openLinear).toHaveBeenCalledExactlyOnceWith(issueUrl));
  expect(getUrl).toHaveBeenCalledExactlyOnceWith(job.id);

  const reload = () => userEvent.click(review.getByRole("button", { name: "Reload preview" }));
  await reload();
  expect(await review.findByText("Plan only")).toBeInTheDocument();
  expect(confirm).toBeDisabled();
  await userEvent.click(confirm);
  expect(approve).not.toHaveBeenCalled();

  await reload();
  expect(await review.findByText(/preview no longer matches this issue/)).toBeInTheDocument();
  expect(confirm).toBeDisabled();
  expect(review.queryByText("Use the revised export format.")).not.toBeInTheDocument();

  await reload();
  expect(await review.findByText("Current conversation")).toBeInTheDocument();
  expect(review.getByText(/The first format was rejected/).textContent).toBe(preview.context?.conversation_history);
  expect(review.getByText("Wiki folder")).toBeInTheDocument();
  expect(review.getByText("~/notes/demo-wiki")).toBeInTheDocument();
  expect(review.getByText("wiki/exports.md#format")).toBeInTheDocument();
  expect(confirm).toBeEnabled();
  await userEvent.click(confirm);
  await waitFor(() => expect(approve).toHaveBeenCalledExactlyOnceWith(job.issue_identifier, "implement-preview"));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});

test("a usual run does not say what it was for", async () => {
  await openRun(12);
  expect(screen.queryByText("Run for", { selector: "dt" })).not.toBeInTheDocument();
  expect(screen.queryByText("Continues", { selector: "dt" })).not.toBeInTheDocument();
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

test.each([
  ["the issue no longer exists in Linear", "The issue was deleted in Linear, so the run was stopped"],
  ["the issue was canceled in Linear", "The issue was canceled in Linear, so the run was stopped"],
  ["the issue was completed in Linear", "The issue was completed in Linear, so the run was stopped"],
  ["the issue was reassigned in Linear", "The issue is no longer assigned to you in Linear, so the run was stopped"],
])("a run stopped because %s says so in the interface's words, the engine's sentence under it", async (reason, headline) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const stopped = (await backend.job(6))!;
  backend.job = vi.fn().mockResolvedValue({ ...stopped, reason });
  await openRun(6, backend);
  // A note, not an alert: the user's own stop is one too.
  const note = (await screen.findByText(headline)).closest<HTMLElement>('[role="status"]')!;
  expect(within(note).getByText(reason)).toBeInTheDocument();
});

test("a stop from Linear reads in the interface language, and a stop by the user keeps its own headline", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const stopped = (await backend.job(6))!;
  backend.job = vi.fn().mockResolvedValue({ ...stopped, reason: "the issue was canceled in Linear" });
  const linear = await openRun(6, backend, "zh-TW");
  expect(screen.getByText("需求在 Linear 上被取消，已停止執行")).toBeInTheDocument();
  linear.unmount();
  await openRun(6);
  expect(screen.getByText("The run was canceled")).toBeInTheDocument();
  expect(screen.getByText("stopped by the user")).toBeInTheDocument();
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

test("a lean run that did not take effect fails like any other run, the engine's words under the headline", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const failed = (await backend.job(8))!;
  // The reason as the engine writes it for a lean run whose init event named another server.
  const reason =
    "lean run did not take effect: the init event lists the MCP servers demo-tools, linear-server instead of only `linear-server`";
  backend.job = vi.fn().mockResolvedValue({ ...failed, reason });
  await openRun(8, backend);
  const note = screen.getByText(reason).closest("[data-slot=alert]") as HTMLElement;
  expect(within(note).getByText("The run failed")).toBeInTheDocument();
  expect(screen.getByRole("alert")).toBe(note);
});

test("the engine's reason is cleaned before it is shown", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const failed = (await backend.job(8))!;
  backend.job = vi.fn().mockResolvedValue({ ...failed, reason: `exit${ESC}[2K code${RLO} 1` });
  await openRun(8, backend);
  expect(screen.getByText("exit code 1")).toBeInTheDocument();
});

// ---- what can be done -------------------------------------------------------------------------

test("a run waiting for dispatch review can be approved from its detail even outside the issues list", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.job(9))!;
  backend.job = vi.fn().mockResolvedValue({ ...job, state: "waiting_binding", reason: "waiting for dispatch review" });
  backend.issues = vi.fn().mockResolvedValue([]);
  await openRun(9, backend);
  expect(screen.getByRole("button", { name: "Preview" })).toBeInTheDocument();
});

test("a run offers what its state allows, beside the title", async () => {
  await openRun(10);
  expect(screen.getByRole("button", { name: "Mark done" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
});

test.each([
  ["matching latest run", true],
  ["issue left testing", false],
  ["a newer run exists", false],
  ["no issue snapshot", false],
  ["Linear read failed", false],
  ["settings read failed", false],
] as const)("local completion on the detail page: %s", async (condition, allowed) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const stored = (await backend.job(10))!;
  const job = { ...stored, state: "local_complete" as const, report: "local_complete" };
  const views = await backend.issues();
  backend.job = vi.fn().mockResolvedValue(job);
  const issue = views.find((view) => view.issue.uuid === job.issue_uuid)!;
  const latest = {
    ...issue,
    issue: condition === "issue left testing" ? { ...issue.issue, state: "Done" } : issue.issue,
    job: condition === "a newer run exists" ? { ...job, id: job.id + 1 } : job,
  };
  if (condition === "settings read failed") {
    backend.settings = vi.fn().mockRejectedValue(new BackendError("cannot read settings"));
  }
  backend.issues = condition === "Linear read failed"
    ? vi.fn().mockRejectedValue(new BackendError("cannot read Linear"))
    : vi.fn().mockResolvedValue(condition === "no issue snapshot" ? [] : [latest]);
  const markDone = vi.fn().mockResolvedValue(undefined);
  backend.markDone = markDone;

  const { client } = await openRun(job.id, backend);
  await waitFor(() => expect(client.getQueryState(queryKeys.issues)?.status).toBe(
    condition === "Linear read failed" ? "error" : "success",
  ));
  await waitFor(() => expect(client.getQueryState(queryKeys.settings)?.status).toBe(
    condition === "settings read failed" ? "error" : "success",
  ));
  expect(screen.getByText("Awaiting acceptance")).toBeInTheDocument();
  if (allowed) {
    await userEvent.click(await screen.findByRole("button", { name: "Mark done" }));
    expect(markDone).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(markDone).toHaveBeenCalledExactlyOnceWith(job.id);
  } else {
    expect(screen.queryByRole("button", { name: "Mark done" })).not.toBeInTheDocument();
    expect(markDone).not.toHaveBeenCalled();
  }
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

test("the model name is not fainter than the muted text around it", async () => {
  await openRun(12);
  const model = await screen.findByText("claude-sonnet-5-5");
  // jsdom has no styles to measure: a fading class is what took this small text under 4.5:1 over the page
  // (4.2:1 in the dark theme, 2.7:1 in the light one; the screenshots' audit measured both).
  expect(model.className).not.toMatch(/opacity/);
  expect(model.parentElement?.className).not.toMatch(/opacity/);
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
  // It has ended: nothing more will come, so not "yet".
  expect(await screen.findByText("This run left nothing to show")).toBeInTheDocument();
  expect(screen.queryByText("Nothing to show yet")).not.toBeInTheDocument();
  expect(screen.queryByText("The log of this run was removed by the log cleanup")).not.toBeInTheDocument();
});

test("a run that ended with events but none worth a row left nothing to show in the timeline", async () => {
  const log = [JSON.stringify({ type: "thread.started", thread_id: "t" }), JSON.stringify({ type: "turn.started" })];
  await openRun(10, backendWithLog(log));
  expect(await screen.findByText("This run left nothing to show")).toBeInTheDocument();
  expect(screen.queryByText("Nothing to show yet")).not.toBeInTheDocument();
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
  expect(within(screen.getByRole("tabpanel", { name: "Timeline" })).getByRole("status", { name: "Loading…" })).toBeInTheDocument();
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

test("a log as long as the panel reads says that only its last lines are shown, in both tabs", async () => {
  // Lines that make no row keep the timeline short; the raw log has them all.
  const lines = Array.from({ length: 5000 }, () => JSON.stringify({ type: "turn.started" }));
  await openRun(10, backendWithLog(lines));
  const note = "Only the last 5,000 lines of the log are shown; the run began before them.";
  expect(await screen.findByText(note)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Raw log" }));
  expect(screen.getByText(note)).toBeInTheDocument();
});

test("a log the panel reads whole says nothing about its length", async () => {
  const lines = [...Array.from({ length: 4998 }, () => JSON.stringify({ type: "turn.started" })), assistant({ type: "text", text: "All done." })];
  await openRun(10, backendWithLog(lines));
  expect(await screen.findByText("All done.")).toBeInTheDocument();
  expect(screen.queryByText(/Only the last/)).not.toBeInTheDocument();
});

test("the files Codex changed are listed with what happened to each", async () => {
  const log = [
    completed({
      type: "file_change",
      changes: [
        { path: "src/lib.rs", kind: "update" },
        { path: "docs/usage.md", kind: "add" },
      ],
      status: "completed",
    }),
  ];
  // DEMO-11 is run by Codex.
  await openRun(11, backendWithLog(log));
  const files = await screen.findByRole("group", { name: "Files changed" });
  expect(within(files).getAllByRole("listitem").map((item) => item.textContent)).toEqual(["Changedsrc/lib.rs", "Addeddocs/usage.md"]);
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

test("a live log that cannot be read again keeps its lines, with the error above them and no flicker", async () => {
  vi.useFakeTimers();
  try {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.jobLog = vi
      .fn()
      .mockResolvedValueOnce([assistant({ type: "text", text: "Working on it." })])
      .mockRejectedValue(new BackendError("cannot read /logs/12.jsonl"));
    await renderWithProviders(page, { backend, route: "/runs/12" });
    await advanceUntil(() => screen.queryByText("Working on it.") !== null);
    await advanceUntil(() => screen.queryByText("Could not read the log") !== null);
    // It is read again every 1.5 s and fails each time: between and during those reads the page
    // shows the same, the lines it had and the error above them.
    for (let round = 0; round < 6; round++) {
      await act(() => vi.advanceTimersByTimeAsync(500));
      expect(screen.getByText("Working on it.")).toBeInTheDocument();
      expect(screen.getByText("Could not read the log")).toBeInTheDocument();
    }
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

describe("a split plan", () => {
  const plans = () => createMockBackend("demo", { latencyMs: 0, plans: true });
  const progress = (patch: Partial<PlanProgress>): PlanProgress => ({
    job_id: 20, issue_uuid: "issue-20", total: 0, completed: 0, canceled: 0, open: 0, closable: false,
    checked_at: "2026-10-12T08:59:00Z", error: null, ...patch,
  });

  test("shows how far its child issues are and offers no implementation of the whole plan", async () => {
    await openRun(20, plans());
    expect(await screen.findByText("Child issues: 3 of 5 ended")).toBeInTheDocument();
    expect(screen.getByText(/This plan was split into child issues\./)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Implement" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Complete plan" })).not.toBeInTheDocument();
    expect(field("Status")).toBe("Plan posted");
  });

  test("whose child issues have all ended is closed after one confirmation", async () => {
    const backend = plans();
    const complete = vi.spyOn(backend, "completePlan");
    await openRun(21, backend);
    await userEvent.click(await screen.findByRole("button", { name: "Complete plan" }));
    expect(field("Status")).toBe("Ready to close");
    const dialog = within(screen.getByRole("alertdialog", { name: "Complete plan" }));
    expect(dialog.getByText(/DEMO-21.*1 completed, 1 canceled.*Done/)).toBeInTheDocument();
    expect(complete).not.toHaveBeenCalled();
    await userEvent.click(dialog.getByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(complete).toHaveBeenCalledExactlyOnceWith(21));
  });

  test("whose child issues cannot be read says so and keeps the implementation action", async () => {
    const backend = plans();
    vi.spyOn(backend, "refreshPlanProgress").mockResolvedValue(progress({ error: "Linear is unreachable" }));
    await openRun(20, backend);
    expect(await screen.findByText("The child issues could not be read.")).toBeInTheDocument();
    expect(screen.getByText("Linear is unreachable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Implement" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Complete plan" })).not.toBeInTheDocument();
  });

  test("whose child issues were all canceled cannot be closed here", async () => {
    const backend = plans();
    vi.spyOn(backend, "refreshPlanProgress").mockResolvedValue(progress({ total: 2, canceled: 2 }));
    await openRun(20, backend);
    expect(await screen.findByText(/Every child issue was canceled/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Complete plan" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Implement" })).not.toBeInTheDocument();
  });

  test("that was closed says when a child issue has not ended since", async () => {
    const backend = plans();
    await backend.completePlan(21);
    vi.spyOn(backend, "refreshPlanProgress").mockResolvedValue(progress({ job_id: 21, total: 3, completed: 1, canceled: 1, open: 1 }));
    await openRun(21, backend);
    expect(await screen.findByText("Some child issues have not ended since the plan was closed (1 open).")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Complete plan" })).not.toBeInTheDocument();
  });
});


import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useLocation } from "react-router";
import { vi } from "vitest";
import { type Backend, BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { IssueStatus, IssueView } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import Issues from "./Issues";

test("each labeled issue says whether it will be picked up", async () => {
  await renderWithProviders(<Issues />);
  expect(await screen.findByText("DEMO-4")).toBeInTheDocument();
  expect(screen.getByText(/status is Backlog, not Todo/)).toBeInTheDocument();
  // Translated from the code `project_not_bound`, not the engine's English sentence.
  expect(screen.getByText(/not bound to a local folder/)).toBeInTheDocument();
});

test("without a Linear connection the list explains what to do", async () => {
  await renderWithProviders(<Issues />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText("Could not read the issues from Linear")).toBeInTheDocument();
  expect(screen.getByText(/not connected to Linear/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Open the guide" })).toBeInTheDocument();
});

test("while the first connection to Linear is under way the page says so, not that it failed", async () => {
  const backend = createMockBackend("connecting", { latencyMs: 0 });
  const { client } = await renderWithProviders(<Issues />, { backend });
  // Announced politely, as a state of things, not as an alert.
  expect((await screen.findByText("Connecting to Linear…")).closest('[role="status"]')).not.toBeNull();
  // The list was asked for and refused ("still connecting"): that is no failure to report.
  await waitFor(() => expect(client.getQueryState(["issues"])?.status).toBe("error"));
  expect(screen.queryByText("Could not read the issues from Linear")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Open the guide" })).not.toBeInTheDocument();
  // Connected: the list is read again (the app's connection refresh does that), and shows.
  await act(async () => {
    await backend.linearReconnect();
    await client.invalidateQueries({ queryKey: ["status"] });
    await client.invalidateQueries({ queryKey: ["issues"] });
  });
  expect(await screen.findByText("DEMO-4")).toBeInTheDocument();
  expect(screen.queryByText("Connecting to Linear…")).not.toBeInTheDocument();
});

/** Shows where the router is, so that a test can follow a button. */
function Where() {
  const location = useLocation();
  return <p data-testid="where">{location.pathname + location.search}</p>;
}

/** The demo backend whose issue list is what `change` makes of the demo's. */
function backendWithIssues(change: (views: IssueView[]) => IssueView[]): Backend {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.issues.bind(backend);
  backend.issues = async () => change(await read());
  return backend;
}

/** The demo backend where DEMO-4 has `status`. */
function backendWhereDemo4Is(status: IssueStatus): Backend {
  return backendWithIssues((views) => views.map((view) => (view.issue.identifier === "DEMO-4" ? { ...view, status } : view)));
}

/** The table row of an issue. */
const rowOf = (identifier: string) => screen.getByText(identifier).closest("tr")!;

test("the list is a table with a column for each thing it says about an issue", async () => {
  await renderWithProviders(<Issues />);
  await screen.findByText("DEMO-4");
  const headers = screen.getAllByRole("columnheader").map((header) => header.textContent);
  expect(headers).toEqual(["Issue", "Title", "Linear status", "Local status", "Reason", "Actions"]);
  // Every issue of the demo is a row, the engine's order kept.
  const rows = screen.getAllByRole("row").slice(1);
  expect(rows.map((row) => within(row).getAllByRole("cell")[0].textContent)).toEqual([
    "DEMO-12",
    "DEMO-11",
    "DEMO-10",
    "DEMO-8",
    "DEMO-7",
    "DEMO-6",
    "DEMO-5",
    "DEMO-4",
    "DEMO-3",
  ]);
});

test("a row has the title, Linear's status and the local state of its newest run", async () => {
  await renderWithProviders(<Issues />);
  await screen.findByText("DEMO-4");
  const withRun = within(rowOf("DEMO-10"));
  expect(withRun.getByText("Add hello.txt")).toBeInTheDocument();
  expect(withRun.getByText("In Testing")).toBeInTheDocument();
  expect(withRun.getByText("Awaiting acceptance")).toBeInTheDocument();
  // An issue that has had no run has a dash where the state would be.
  const withoutRun = within(rowOf("DEMO-4"));
  expect(withoutRun.getByText("Migrate the build to the new runner")).toBeInTheDocument();
  expect(withoutRun.getByText("Backlog")).toBeInTheDocument();
  expect(withoutRun.getByText("—")).toBeInTheDocument();
});

test("each row offers what its run allows, and the actions column never holds a second button", async () => {
  await renderWithProviders(<Issues />);
  await screen.findByText("DEMO-4");
  expect(within(rowOf("DEMO-10")).getByRole("button", { name: "Mark done" })).toBeInTheDocument();
  expect(within(rowOf("DEMO-12")).getByRole("button", { name: "Stop" })).toBeInTheDocument();
  expect(within(rowOf("DEMO-8")).getByRole("button", { name: "Run again" })).toBeInTheDocument();
  // At the smallest window the table has about 600 px: a row's actions are one button at most (an
  // issue without a run has none), because Open in Linear is the identifier and not another button.
  for (const row of screen.getAllByRole("row").slice(1)) {
    const actions = within(row).getAllByRole("cell").at(-1)!;
    expect(within(actions).queryAllByRole("button").length, row.textContent ?? "").toBeLessThanOrEqual(1);
  }
  expect(within(within(rowOf("DEMO-4")).getAllByRole("cell").at(-1)!).queryAllByRole("button")).toHaveLength(0);
});

test("the identifier of an issue is what opens it in Linear, through the backend, with the issue's link", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const open = vi.spyOn(backend, "openLinear");
  await renderWithProviders(<Issues />, { backend });
  await screen.findByText("DEMO-4");
  const identifier = within(rowOf("DEMO-4")).getByRole("button", { name: "Open DEMO-4 in Linear" });
  expect(identifier).toHaveTextContent("DEMO-4");
  expect(within(rowOf("DEMO-4")).getAllByRole("cell")[0]).toContainElement(identifier);
  await userEvent.click(identifier);
  expect(open).toHaveBeenCalledWith("https://linear.app/demo/issue/DEMO-4");
});

test("marking an issue's run done asks first, from the issue list too", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const markDone = vi.spyOn(backend, "markDone");
  await renderWithProviders(<Issues />, { backend });
  await screen.findByText("DEMO-4");
  await userEvent.click(within(rowOf("DEMO-10")).getByRole("button", { name: "Mark done" }));
  expect(markDone).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(markDone).toHaveBeenCalledWith(10);
});

test("every verdict of the engine is said in the interface's words", async () => {
  const cases: [IssueStatus, string][] = [
    [{ kind: "eligible", picked_up: true }, "The next check picks it up"],
    [{ kind: "eligible", picked_up: false }, "It runs again after a change in Linear"],
    [{ kind: "not_candidate" }, "It is not assigned to you, or its label does not match"],
    [{ kind: "paused" }, "Monitoring is paused"],
    [{ kind: "waiting", reason: { code: "project_not_bound" } }, "Its project is not bound to a local folder"],
    [{ kind: "waiting", reason: { code: "directory_missing", directory: "~/code/missing" } }, "The bound folder ~/code/missing does not exist"],
    [{ kind: "waiting", reason: { code: "executor_not_ready", executor: "codex" } }, "Codex is not ready; see the environment check"],
    [{ kind: "waiting", reason: { code: "executor_not_ready", executor: "claude" } }, "Claude Code is not ready; see the environment check"],
    [{ kind: "waiting", reason: { code: "lean_not_ready", executor: "claude" } }, "Claude Code is not ready for lean runs; see the environment check"],
    [{ kind: "waiting", reason: { code: "lean_not_ready", executor: "codex" } }, "Codex is not ready for lean runs; see the environment check"],
    [{ kind: "not_todo", state: "In Review" }, "Its status is In Review, not Todo"],
  ];
  for (const [status, text] of cases) {
    const view = await renderWithProviders(<Issues />, { backend: backendWhereDemo4Is(status) });
    await screen.findByText("DEMO-4");
    expect(within(rowOf("DEMO-4")).getByText(text), text).toBeInTheDocument();
    view.unmount();
  }
});

test("an eligible issue whose run ended is not picked up by itself, and the row says so beside Run again", async () => {
  await renderWithProviders(<Issues />);
  await screen.findByText("DEMO-4");
  // DEMO-6's run was stopped and nothing changed in Linear since: only Run again starts it.
  const stopped = within(rowOf("DEMO-6"));
  expect(stopped.getByText("It runs again after a change in Linear")).toBeInTheDocument();
  expect(stopped.queryByText("The next check picks it up")).not.toBeInTheDocument();
  expect(stopped.getByRole("button", { name: "Run again" })).toBeInTheDocument();
  // An issue that has had no run, and one whose run is queued, are picked up.
  expect(within(rowOf("DEMO-3")).getByText("The next check picks it up")).toBeInTheDocument();
  expect(within(rowOf("DEMO-11")).getByText("The next check picks it up")).toBeInTheDocument();
});

test("stopping a queued run turns its row from picked up to not picked up", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await renderWithProviders(<Issues />, { backend });
  await screen.findByText("DEMO-4");
  expect(within(rowOf("DEMO-11")).getByText("The next check picks it up")).toBeInTheDocument();
  await userEvent.click(within(rowOf("DEMO-11")).getByRole("button", { name: "Stop" }));
  expect(await within(rowOf("DEMO-11")).findByText("It runs again after a change in Linear")).toBeInTheDocument();
  expect(within(rowOf("DEMO-11")).getByRole("button", { name: "Run again" })).toBeInTheDocument();
});

test.each(["queued", "waiting_binding", "running"] as const)("a %s run says it all: the status of its issue is not repeated", async (state) => {
  // DEMO-4 has no run in the demo and is in Backlog: give it DEMO-12's run, in the state asked for.
  const backend = backendWithIssues((views) => {
    const run = views.find((view) => view.issue.identifier === "DEMO-12")!.job!;
    return views.map((view) => (view.issue.identifier === "DEMO-4" ? { ...view, job: { ...run, state } } : view));
  });
  await renderWithProviders(<Issues />, { backend });
  await screen.findByText("DEMO-4");
  expect(within(rowOf("DEMO-4")).queryByText(/Its status is/)).not.toBeInTheDocument();
});

test("an issue whose run has ended still says what status stops the next check", async () => {
  await renderWithProviders(<Issues />);
  await screen.findByText("DEMO-4");
  // DEMO-8 failed and is In Progress; DEMO-12 is In Progress with its run running (shows nothing).
  expect(within(rowOf("DEMO-8")).getByText("Its status is In Progress, not Todo")).toBeInTheDocument();
  expect(within(rowOf("DEMO-12")).queryByText(/Its status is/)).not.toBeInTheDocument();
  expect(within(rowOf("DEMO-10")).getByText("Its status is In Testing, not Todo")).toBeInTheDocument();
  expect(within(rowOf("DEMO-4")).getByText("Its status is Backlog, not Todo")).toBeInTheDocument();
});

test("the engine's codes are never shown as they are", async () => {
  await renderWithProviders(<Issues />, { backend: backendWhereDemo4Is({ kind: "waiting", reason: { code: "directory_missing", directory: "~/code/missing" } }) });
  await screen.findByText("DEMO-4");
  expect(screen.queryByText(/directory_missing|project_not_bound|executor_not_ready|lean_not_ready/)).not.toBeInTheDocument();
});

test("a folder name from the configuration is shown as text and cleaned", async () => {
  const esc = String.fromCharCode(27);
  const directory = `~/code/<b>odd</b>${esc}[0m`;
  await renderWithProviders(<Issues />, { backend: backendWhereDemo4Is({ kind: "waiting", reason: { code: "directory_missing", directory } }) });
  await screen.findByText("DEMO-4");
  expect(within(rowOf("DEMO-4")).getByText("The bound folder ~/code/<b>odd</b> does not exist")).toBeInTheDocument();
});

test("the reasons name the statuses the settings configure", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const settings = await backend.settings();
  backend.settings = async () => ({ ...settings, trigger: { ...settings.trigger, todo: "Ready to build", label: "bot-ready" } });
  await renderWithProviders(<Issues />, { backend });
  expect(await screen.findByText(/Issues assigned to you with the bot-ready label/)).toBeInTheDocument();
  expect(await screen.findByText("Its status is Backlog, not Ready to build")).toBeInTheDocument();
});

test("the header says which issues are listed", async () => {
  await renderWithProviders(<Issues />);
  expect(screen.getByRole("heading", { name: "Issues" })).toBeInTheDocument();
  expect(await screen.findByText("Issues assigned to you with the ai-ready label, the 50 most recently updated")).toBeInTheDocument();
});

test("the refresh button reads the issues from Linear again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = vi.spyOn(backend, "issues");
  await renderWithProviders(<Issues />, { backend });
  await screen.findByText("DEMO-4");
  expect(read).toHaveBeenCalledTimes(1);
  await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
  expect(read).toHaveBeenCalledTimes(2);
});

test("while the issues are read, five placeholder rows stand in", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.issues = () => new Promise<IssueView[]>(() => {});
  const { container } = await renderWithProviders(<Issues />, { backend });
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(container.querySelectorAll("[data-slot=skeleton]")).toHaveLength(5);
  expect(screen.queryByRole("table")).not.toBeInTheDocument();
});

test("with no labeled issue the page says how an issue gets here, and where to read more", async () => {
  await renderWithProviders(
    <>
      <Issues />
      <Where />
    </>,
    { backend: backendWithIssues(() => []) },
  );
  expect(await screen.findByText("No issues with the ai-ready label yet")).toBeInTheDocument();
  expect(screen.getByText(/assigned to you, has the ai-ready label and is in Todo/)).toBeInTheDocument();
  expect(screen.queryByRole("table")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "How it works" }));
  expect(screen.getByTestId("where")).toHaveTextContent("/help");
});

test("the empty list names the label and the status the settings configure", async () => {
  const backend = backendWithIssues(() => []);
  const settings = await backend.settings();
  backend.settings = async () => ({ ...settings, trigger: { ...settings.trigger, todo: "Ready to build", label: "bot-ready" } });
  await renderWithProviders(<Issues />, { backend });
  expect(await screen.findByText("No issues with the bot-ready label yet")).toBeInTheDocument();
  expect(screen.getByText(/has the bot-ready label and is in Ready to build/)).toBeInTheDocument();
});

test("the error state leads to the guide and to the environment check", async () => {
  await renderWithProviders(
    <>
      <Issues />
      <Where />
    </>,
    { backend: createMockBackend("fresh", { latencyMs: 0 }) },
  );
  await userEvent.click(await screen.findByRole("button", { name: "Open the guide" }));
  expect(screen.getByTestId("where")).toHaveTextContent("/onboarding");
  await userEvent.click(screen.getByRole("button", { name: "Open the environment check" }));
  expect(screen.getByTestId("where")).toHaveTextContent("/doctor");
});

test("a retry in the error state reads the issues again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.issues.bind(backend);
  backend.issues = vi.fn().mockRejectedValueOnce(new BackendError("Linear did not answer")).mockImplementation(read);
  await renderWithProviders(<Issues />, { backend });
  expect(await screen.findByText("Could not read the issues from Linear")).toBeInTheDocument();
  expect(screen.getByText("Linear did not answer")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByText("DEMO-4")).toBeInTheDocument();
  expect(screen.queryByText("Could not read the issues from Linear")).not.toBeInTheDocument();
});

test("the issue list reads in the interface language", async () => {
  await renderWithProviders(<Issues />, { lang: "zh-CN" });
  expect(await screen.findByText("DEMO-4")).toBeInTheDocument();
  expect(screen.getByText("状态是 Backlog，不是 Todo")).toBeInTheDocument();
  expect(screen.getByText("项目还没有绑定本地目录")).toBeInTheDocument();
  expect(screen.getByText("指派给你、带 ai-ready 标签的需求，最近更新的 50 条")).toBeInTheDocument();
});

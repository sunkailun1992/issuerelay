import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { queryKeys } from "@/hooks/queries";
import { createMockBackend } from "@/lib/mock";
import { DEMO_DOCTOR } from "@/lib/mock-fixtures";
import type { ExecutorReport } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { ExecutorCheck } from "./ExecutorCheck";

const [claude, codex] = DEMO_DOCTOR.executors;

/** `report` with `change` applied. */
const with_ = (report: ExecutorReport, change: Partial<ExecutorReport>): ExecutorReport => ({ ...report, ...change });

/** The fix the guide and the environment check both offer when Linear is not connected in `report`. */
const unconnected = (report: ExecutorReport) => with_(report, { linear_mcp: false, problems: ["Linear MCP: Needs authentication"] });

const notInstalled = (report: ExecutorReport) =>
  with_(report, { program: null, version: null, logged_in: false, linear_mcp: false, problems: ["not installed"] });

/** The symbol in front of a line of the card: the colour it has says whether the check passed. */
function symbolOf(card: HTMLElement, text: string): string {
  return within(card).getByText(text).closest("li")?.querySelector("svg")?.getAttribute("class") ?? "";
}

test("each check has a symbol in the colour of its result", async () => {
  await renderWithProviders(<ExecutorCheck report={with_(claude, { logged_in: false, linear_mcp: false, problems: [] })} onFixed={() => {}} />);
  const card = screen.getByRole("region", { name: "Claude Code" });
  expect(symbolOf(card, "Installed")).toContain("text-state-done");
  expect(symbolOf(card, "Not logged in")).toContain("text-state-failed");
  expect(symbolOf(card, "Linear MCP is not connected")).toContain("text-state-failed");
});

test("an executor that is not installed has nothing to log in to, and says how to get it", async () => {
  await renderWithProviders(<ExecutorCheck report={notInstalled(codex)} onFixed={() => {}} />);
  const card = screen.getByRole("region", { name: "Codex" });
  expect(symbolOf(card, "Not installed")).toContain("text-state-failed");
  expect(within(card).queryByText(/in a terminal and log in/)).not.toBeInTheDocument();
  expect(within(card).getByText("https://developers.openai.com/codex/cli")).toBeInTheDocument();
});

test("a ready executor shows its three checks, its version and where it was found, and nothing to do", async () => {
  await renderWithProviders(<ExecutorCheck report={claude} onFixed={() => {}} />);
  const card = screen.getByRole("region", { name: "Claude Code" });
  expect(within(card).getByText("Installed")).toBeInTheDocument();
  expect(within(card).getByText("2.1.285 (Claude Code)")).toBeInTheDocument();
  expect(within(card).getByText("~/.local/bin/claude")).toBeInTheDocument();
  expect(within(card).getByText("Logged in")).toBeInTheDocument();
  expect(within(card).getByText("Linear MCP is connected")).toBeInTheDocument();
  expect(within(card).queryByRole("button")).not.toBeInTheDocument();
});

// The first-run guide finds a step by the executor's name: it must be in one element only, the
// card's heading, whatever state the card is in.
test.each([
  ["a ready executor", claude],
  ["an executor that is not logged in", with_(claude, { logged_in: false, problems: ["claude is not logged in"] })],
  ["an executor without Linear", unconnected(claude)],
  ["an executor that is not installed", notInstalled(claude)],
])("the executor's name stands in one element only for %s", async (_state, report) => {
  await renderWithProviders(<ExecutorCheck report={report} onFixed={() => {}} />);
  await screen.findByRole("region", { name: "Claude Code" });
  // The fixes are also named with it ("Add Linear to Claude Code"), but never as the whole text.
  expect(screen.getAllByText("Claude Code")).toHaveLength(1);
});

test("the name Codex stands in one element only, too", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.linearServerNamed = vi.fn().mockResolvedValue(false);
  await renderWithProviders(<ExecutorCheck report={unconnected(codex)} onFixed={() => {}} />, { backend });
  await screen.findByRole("button", { name: "Add Linear to Codex" });
  expect(screen.getAllByText("Codex")).toHaveLength(1);
});

test("onFixed is called once a fix has worked", async () => {
  const onFixed = vi.fn();
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<ExecutorCheck report={unconnected(claude)} onFixed={onFixed} />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" }));
  expect(setup).toHaveBeenCalledWith({ kind: "login_linear_mcp", executor: "claude" });
  await waitFor(() => expect(onFixed).toHaveBeenCalledTimes(1));
});

test("onFixed is not called when the fix failed", async () => {
  const onFixed = vi.fn();
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.setupAction = vi.fn().mockRejectedValue(new BackendError("the browser was closed"));
  await renderWithProviders(<ExecutorCheck report={unconnected(claude)} onFixed={onFixed} />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" }));
  expect(await screen.findByText("The setup command failed")).toBeInTheDocument();
  expect(onFixed).not.toHaveBeenCalled();
});

test("onFixed is optional: the setup action itself makes the check outdated wherever it is shown", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const { client } = await renderWithProviders(<ExecutorCheck report={unconnected(claude)} />, { backend });
  // A check that nothing shows (the guide's, before its step is open) is only marked outdated.
  await client.fetchQuery({ queryKey: queryKeys.doctor, queryFn: () => backend.doctor() });
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" }));
  await waitFor(() => expect(client.getQueryState(queryKeys.doctor)?.isInvalidated).toBe(true));
});

test("an executor that knows Linear under another name gets the add button, not the login", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.linearServerNamed = vi.fn().mockResolvedValue(false);
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<ExecutorCheck report={unconnected(codex)} onFixed={() => {}} />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Add Linear to Codex" }));
  expect(setup).toHaveBeenCalledWith({ kind: "add_linear_mcp", executor: "codex" });
  expect(screen.queryByRole("button", { name: "Authorize Linear in Codex" })).not.toBeInTheDocument();
});

test("the engine's own lines are detail lines, one each, never the headline", async () => {
  const report = with_(claude, { logged_in: false, problems: ["first problem", "second problem"] });
  await renderWithProviders(<ExecutorCheck report={report} onFixed={() => {}} />);
  const card = screen.getByRole("region", { name: "Claude Code" });
  expect(within(card).getByText("first problem")).toBeInTheDocument();
  expect(within(card).getByText("second problem")).toBeInTheDocument();
  expect(within(card).getByText("Not logged in")).toBeInTheDocument();
});

test("control characters in what the executor reported are cleaned before they are shown", async () => {
  const report = with_(claude, {
    version: `2.1.285 ${String.fromCharCode(27)}[31m(Claude Code)${String.fromCharCode(27)}[0m`,
    problems: [`bad ${String.fromCharCode(27)}[2Jline`],
  });
  await renderWithProviders(<ExecutorCheck report={report} onFixed={() => {}} />);
  const card = screen.getByRole("region", { name: "Claude Code" });
  expect(within(card).getByText("2.1.285 (Claude Code)")).toBeInTheDocument();
  expect(within(card).getByText("bad line")).toBeInTheDocument();
});

test("an installed executor whose version could not be read is still installed", async () => {
  await renderWithProviders(<ExecutorCheck report={with_(claude, { version: null })} onFixed={() => {}} />);
  const card = screen.getByRole("region", { name: "Claude Code" });
  expect(within(card).getByText("Installed")).toBeInTheDocument();
  expect(within(card).queryByText(/Claude Code\)/)).not.toBeInTheDocument();
});

test("the lean part is shown only where it is asked for: the environment check, not the guide", async () => {
  const { unmount } = await renderWithProviders(<ExecutorCheck report={claude} onFixed={() => {}} />);
  const card = screen.getByRole("region", { name: "Claude Code" });
  expect(within(card).queryByRole("region", { name: "Lean runs" })).not.toBeInTheDocument();
  unmount();
  await renderWithProviders(<ExecutorCheck report={claude} lean />);
  const checked = screen.getByRole("region", { name: "Claude Code" });
  expect(within(checked).getByRole("region", { name: "Lean runs" })).toHaveTextContent("Ready");
});

test("an executor that is not installed is not ready for lean runs, whatever the lean checks say", async () => {
  await renderWithProviders(<ExecutorCheck report={with_(notInstalled(claude), { lean_linear_mcp: true, lean_logged_in: true })} lean />);
  const lean = screen.getByRole("region", { name: "Lean runs" });
  expect(within(lean).getByText("Not ready")).toBeInTheDocument();
  expect(symbolOf(lean, "Not ready")).toContain("text-state-failed");
});

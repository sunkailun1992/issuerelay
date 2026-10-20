import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes } from "react-router";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { DoctorReport, ExecutorReport } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import Doctor from "./Doctor";

test("the check shows each executor and offers the fix it needs", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  expect(await screen.findByText(/2\.1\.285/)).toBeInTheDocument();
  // The engine's problem text is a detail line under a translated check.
  expect(screen.getByText("codex is not logged in")).toBeInTheDocument();
  expect(screen.getByText(/codex login/)).toBeInTheDocument();
  expect(screen.getByText("HTTPS_PROXY=http://127.0.0.1:7890")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Run the first-run guide again" })).toBeInTheDocument();
  expect(setup).not.toHaveBeenCalled();
});

test("Linear registered under another name gets the add button and the manual login", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  report.executors[0] = { ...report.executors[0], linear_mcp: false, problems: ["Linear MCP is not configured or not authorized in claude"] };
  const doctor = vi.spyOn(backend, "doctor").mockResolvedValueOnce(report);
  backend.linearServerNamed = vi.fn().mockResolvedValue(false);
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  const add = await screen.findByRole("button", { name: "Add Linear to Claude Code" });
  // Both checks are made before the click: the check that follows a fix is a local call that
  // answers at once, so it can land within the very click that started it, and after the click the
  // card may already show the new report, in which Linear is connected. What matters is the card
  // that offers the add button and the manual way, and not the one-click login.
  expect(screen.queryByRole("button", { name: "Authorize Linear in Claude Code" })).not.toBeInTheDocument();
  expect(screen.getByText(/claude mcp list/)).toBeInTheDocument();
  await userEvent.click(add);
  expect(setup).toHaveBeenCalledWith({ kind: "add_linear_mcp", executor: "claude" });
  await waitFor(() => expect(doctor).toHaveBeenCalledTimes(2));
});

test("Linear registered under the default name gets the one-click browser login", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  report.executors[0] = { ...report.executors[0], linear_mcp: false, problems: ["Linear MCP in claude: Needs authentication"] };
  vi.spyOn(backend, "doctor").mockResolvedValueOnce(report);
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" }));
  expect(setup).toHaveBeenCalledWith({ kind: "login_linear_mcp", executor: "claude" });
});

test("when the check cannot run the page says so and offers a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.doctor = vi.fn().mockRejectedValue(new BackendError("IssueRelay could not start"));
  await renderWithProviders(<Doctor />, { backend });
  expect(await screen.findByText("Could not run the environment check")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});

// ---- helpers ----------------------------------------------------------------------------------

/** The demo backend whose environment check answers with the demo report as `change` leaves it. */
async function demoWith(change: (report: DoctorReport) => void) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  change(report);
  backend.doctor = vi.fn().mockResolvedValue(report);
  return backend;
}

/** Changes the executor at `index` of the report (0 is Claude Code, 1 is Codex). */
function changeExecutor(report: DoctorReport, index: number, change: Partial<ExecutorReport>) {
  report.executors[index] = { ...report.executors[index], ...change };
}

/** The symbol in front of a line of a check: the colour it has says whether the check passed. */
function symbolOf(container: HTMLElement, text: string): string {
  const row = within(container).getByText(text).closest("li");
  return row?.querySelector("svg")?.getAttribute("class") ?? "";
}

/** The clipboard of the test: what is written to it. Removed again after each test. */
function stubClipboard(writeText = vi.fn().mockResolvedValue(undefined)) {
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  return writeText;
}
afterEach(() => {
  Reflect.deleteProperty(navigator, "clipboard");
});

// ---- the page and its five sections -----------------------------------------------------------

test("the page is five sections, each named by its heading", async () => {
  await renderWithProviders(<Doctor />);
  expect(screen.getByRole("heading", { level: 1, name: "Environment" })).toBeInTheDocument();
  for (const name of ["Linear", "Workspace", "Executors", "Runtime environment", "Bindings"]) {
    expect(await screen.findByRole("region", { name })).toBeInTheDocument();
  }
  // Each executor card is a section of its own, named for the executor.
  expect(screen.getByRole("region", { name: "Claude Code" })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Codex" })).toBeInTheDocument();
});

test("opening the page runs the check once, and Check again runs it again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const doctor = vi.spyOn(backend, "doctor");
  const { client } = await renderWithProviders(<Doctor />, { backend });
  await screen.findByText(/2\.1\.285/);
  await waitFor(() => expect(client.isFetching()).toBe(0));
  expect(doctor).toHaveBeenCalledTimes(1);
  await userEvent.click(screen.getByRole("button", { name: "Check again" }));
  await waitFor(() => expect(doctor).toHaveBeenCalledTimes(2));
});

test("while the check runs the page says how long it may take and keeps the last report in view", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  let release: ((value: DoctorReport) => void) | undefined;
  backend.doctor = vi
    .fn()
    .mockResolvedValueOnce(report)
    .mockImplementationOnce(() => new Promise<DoctorReport>((resolve) => (release = resolve)));
  await renderWithProviders(<Doctor />, { backend });
  await screen.findByText(/2\.1\.285/);
  await waitFor(() => expect(screen.queryByText(/This takes up to a minute/)).not.toBeInTheDocument());

  await userEvent.click(screen.getByRole("button", { name: "Check again" }));
  expect(await screen.findByText("Checking Linear, Claude Code and Codex. This takes up to a minute.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Check again" })).toBeDisabled();
  // The report from before stays on the page while the new one is read.
  expect(screen.getByText(/2\.1\.285/)).toBeInTheDocument();

  release?.(report);
  await waitFor(() => expect(screen.queryByText(/This takes up to a minute/)).not.toBeInTheDocument());
  expect(screen.getByRole("button", { name: "Check again" })).toBeEnabled();
});

test("the first check shows a loading state with the same note", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  let release: ((value: DoctorReport) => void) | undefined;
  backend.doctor = vi.fn(() => new Promise<DoctorReport>((resolve) => (release = resolve)));
  await renderWithProviders(<Doctor />, { backend });
  expect(screen.getByText(/This takes up to a minute/)).toBeInTheDocument();
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Executors" })).not.toBeInTheDocument();
  release?.(report);
  expect(await screen.findByRole("region", { name: "Executors" })).toBeInTheDocument();
});

test("a check that failed can be tried again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const answer = backend.doctor;
  backend.doctor = vi.fn(answer).mockRejectedValueOnce(new BackendError("IssueRelay could not start"));
  await renderWithProviders(<Doctor />, { backend });
  expect(await screen.findByText("Could not run the environment check")).toBeInTheDocument();
  // The reason the engine gave is the detail under the translated headline.
  expect(screen.getByText("IssueRelay could not start")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByText(/2\.1\.285/)).toBeInTheDocument();
  expect(screen.queryByText("Could not run the environment check")).not.toBeInTheDocument();
});

test("the guide can be run again from the page", async () => {
  await renderWithProviders(
    <Routes>
      <Route path="/" element={<Doctor />} />
      <Route path="/onboarding" element={<p>the guide</p>} />
    </Routes>,
  );
  await userEvent.click(await screen.findByRole("link", { name: "Run the first-run guide again" }));
  expect(await screen.findByText("the guide")).toBeInTheDocument();
});

test("the link to the guide and the retry stay on the page when the check failed", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.doctor = vi.fn().mockRejectedValue(new BackendError("IssueRelay could not start"));
  await renderWithProviders(<Doctor />, { backend });
  await screen.findByText("Could not run the environment check");
  expect(screen.getByRole("link", { name: "Run the first-run guide again" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { level: 1, name: "Environment" })).toBeInTheDocument();
});

// ---- Linear -----------------------------------------------------------------------------------

test("Linear's block is the connection card, with the buttons that manage the connection", async () => {
  await renderWithProviders(<Doctor />);
  const linear = await screen.findByRole("region", { name: "Linear" });
  expect(await within(linear).findByText("Jane Doe")).toBeInTheDocument();
  expect(within(linear).getByRole("button", { name: "Authorize again" })).toBeInTheDocument();
  expect(within(linear).getByRole("button", { name: "Log out" })).toBeInTheDocument();
  // Nothing is wrong with Linear: the failed-check headline is not there.
  expect(within(linear).queryByText("The check of Linear failed")).not.toBeInTheDocument();
});

test("a Linear that could not be checked says so under the connection, in the engine's words", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await renderWithProviders(<Doctor />, { backend });
  const linear = await screen.findByRole("region", { name: "Linear" });
  expect(await within(linear).findByRole("button", { name: "Log in to Linear" })).toBeInTheDocument();
  expect(await within(linear).findByText("The check of Linear failed")).toBeInTheDocument();
  expect(within(linear).getByText("not connected to Linear: log in first")).toBeInTheDocument();
});

test("logging in on the page makes the check run again and fills in the workspace", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await renderWithProviders(<Doctor />, { backend });
  expect(await screen.findByText("Connect to Linear first to check the workspace")).toBeInTheDocument();
  await userEvent.click(await screen.findByRole("button", { name: "Log in to Linear" }));
  expect(await screen.findByText("The label ai-ready exists")).toBeInTheDocument();
  expect(screen.queryByText("Connect to Linear first to check the workspace")).not.toBeInTheDocument();
  expect(screen.queryByText("The check of Linear failed")).not.toBeInTheDocument();
});

// ---- the workspace ----------------------------------------------------------------------------

test("a complete workspace needs nothing from Linear", async () => {
  await renderWithProviders(<Doctor />);
  const workspace = await screen.findByRole("region", { name: "Workspace" });
  expect(await within(workspace).findByText("The label ai-ready exists")).toBeInTheDocument();
  expect(within(workspace).getByText("The statuses IssueRelay uses exist")).toBeInTheDocument();
  expect(within(workspace).getByText("Teams checked: Demo")).toBeInTheDocument();
  expect(within(workspace).queryByRole("button", { name: "Open Linear" })).not.toBeInTheDocument();
});

test("the label that is looked for is the one in the settings", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const settings = await backend.settings();
  backend.settings = async () => ({ ...settings, trigger: { ...settings.trigger, label: "ready-for-ai" } });
  await renderWithProviders(<Doctor />, { backend });
  expect(await screen.findByText("The label ready-for-ai exists")).toBeInTheDocument();
});

test("a missing label and missing statuses are explained, with a button that opens Linear", async () => {
  const backend = await demoWith((report) => {
    report.workspace = { label_ok: false, missing_states: ["In Progress", "In Testing"], teams_checked: ["Demo", "Docs"] };
  });
  const open = vi.spyOn(backend, "openLinear");
  await renderWithProviders(<Doctor />, { backend });
  const workspace = await screen.findByRole("region", { name: "Workspace" });
  expect(await within(workspace).findByText("The label ai-ready is missing")).toBeInTheDocument();
  expect(within(workspace).getByText("Missing statuses: In Progress, In Testing")).toBeInTheDocument();
  expect(within(workspace).getByText("Teams checked: Demo, Docs")).toBeInTheDocument();
  expect(within(workspace).getByText(/Settings → Teams → Workflow/)).toBeInTheDocument();
  expect(within(workspace).getByText(/Settings → Workspace → Labels/)).toBeInTheDocument();
  // The program does not create them: it only points the way.
  await userEvent.click(within(workspace).getByRole("button", { name: "Open Linear" }));
  expect(open).toHaveBeenCalledWith("https://linear.app/");
});

test("with only a status missing, only the statuses are explained", async () => {
  const backend = await demoWith((report) => {
    report.workspace = { label_ok: true, missing_states: ["In Testing"], teams_checked: ["Demo"] };
  });
  await renderWithProviders(<Doctor />, { backend });
  const workspace = await screen.findByRole("region", { name: "Workspace" });
  expect(await within(workspace).findByText("Missing statuses: In Testing")).toBeInTheDocument();
  expect(within(workspace).getByText("The label ai-ready exists")).toBeInTheDocument();
  expect(within(workspace).getByText(/Settings → Teams → Workflow/)).toBeInTheDocument();
  expect(within(workspace).queryByText(/Settings → Workspace → Labels/)).not.toBeInTheDocument();
});

test("a workspace that could not be checked is an empty state, not a green check", async () => {
  const backend = await demoWith((report) => {
    report.workspace = null;
  });
  await renderWithProviders(<Doctor />, { backend });
  const workspace = await screen.findByRole("region", { name: "Workspace" });
  expect(await within(workspace).findByText("Connect to Linear first to check the workspace")).toBeInTheDocument();
  expect(within(workspace).queryByText(/The label/)).not.toBeInTheDocument();
});

// ---- the executors ----------------------------------------------------------------------------

test("each executor shows its three checks as translated titles with the engine's lines below", async () => {
  await renderWithProviders(<Doctor />);
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  expect(await within(claude).findByText("Installed")).toBeInTheDocument();
  expect(within(claude).getByText("Logged in")).toBeInTheDocument();
  expect(within(claude).getByText("Linear MCP is connected")).toBeInTheDocument();
  expect(within(claude).queryByText("codex is not logged in")).not.toBeInTheDocument();

  const codex = screen.getByRole("region", { name: "Codex" });
  expect(within(codex).getByText("Installed")).toBeInTheDocument();
  expect(within(codex).getByText("Not logged in")).toBeInTheDocument();
  expect(within(codex).getByText("Linear MCP is connected")).toBeInTheDocument();
  expect(within(codex).getByText("codex is not logged in")).toBeInTheDocument();
  expect(within(codex).getByText("codex login")).toBeInTheDocument();
});

test("an executor that is not logged in says which command logs it in", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 0, { logged_in: false, problems: ["claude is not logged in"] });
  });
  await renderWithProviders(<Doctor />, { backend });
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  expect(await within(claude).findByText("Not logged in")).toBeInTheDocument();
  expect(within(claude).getByText("claude")).toBeInTheDocument();
  expect(within(claude).getByText(/in a terminal and log in/)).toBeInTheDocument();
});

test("an executor that is not installed shows the official address to copy, and opens nothing", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 1, {
      program: null,
      version: null,
      logged_in: false,
      linear_mcp: false,
      problems: ["`codex` is not installed or not on PATH"],
    });
  });
  const open = vi.spyOn(backend, "openLinear");
  const writeText = stubClipboard();
  await renderWithProviders(<Doctor />, { backend });
  const codex = await screen.findByRole("region", { name: "Codex" });
  expect(await within(codex).findByText("Not installed")).toBeInTheDocument();
  expect(within(codex).getByText("`codex` is not installed or not on PATH")).toBeInTheDocument();
  expect(within(codex).getByText("https://developers.openai.com/codex/cli")).toBeInTheDocument();
  // Nothing to log in to before it is installed.
  expect(within(codex).queryByText(/in a terminal and log in/)).not.toBeInTheDocument();
  await userEvent.click(within(codex).getByRole("button", { name: "Copy" }));
  expect(writeText).toHaveBeenCalledWith("https://developers.openai.com/codex/cli");
  expect(await screen.findByText("Copied")).toBeInTheDocument();
  // The address is text to copy: the app opens only Linear's pages.
  expect(open).not.toHaveBeenCalled();
});

test("Claude Code's official address is its own", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 0, { program: null, version: null, logged_in: false, linear_mcp: false, problems: [] });
  });
  await renderWithProviders(<Doctor />, { backend });
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  expect(await within(claude).findByText("https://docs.anthropic.com/en/docs/claude-code/setup")).toBeInTheDocument();
});

test("an executor that is not installed is neither asked about Linear nor offered a way to connect it", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 0, { linear_mcp: false, problems: ["Linear MCP in claude: Needs authentication"] });
    changeExecutor(report, 1, { program: null, version: null, logged_in: false, linear_mcp: false, problems: [] });
  });
  const named = vi.spyOn(backend, "linearServerNamed");
  await renderWithProviders(<Doctor />, { backend });
  const codex = await screen.findByRole("region", { name: "Codex" });
  expect(await within(codex).findByText("Not installed")).toBeInTheDocument();
  expect(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" })).toBeInTheDocument();
  expect(within(codex).queryByRole("button", { name: /Linear/ })).not.toBeInTheDocument();
  expect(named).toHaveBeenCalledTimes(1);
  expect(named).toHaveBeenCalledWith("claude");
});

test("an executor that is ready has nothing to fix, and Linear is not asked about", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 1, { logged_in: true, problems: [] });
  });
  const named = vi.spyOn(backend, "linearServerNamed");
  await renderWithProviders(<Doctor />, { backend });
  const codex = await screen.findByRole("region", { name: "Codex" });
  expect(await within(codex).findByText("Logged in")).toBeInTheDocument();
  expect(within(codex).queryByRole("button")).not.toBeInTheDocument();
  expect(named).not.toHaveBeenCalled();
});

test("the codex commands are codex's own", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 1, { linear_mcp: false, problems: ["Linear MCP in codex: Not logged in"] });
  });
  backend.linearServerNamed = vi.fn().mockResolvedValue(false);
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Doctor />, { backend });
  const codex = await screen.findByRole("region", { name: "Codex" });
  await userEvent.click(await within(codex).findByRole("button", { name: "Add Linear to Codex" }));
  expect(setup).toHaveBeenCalledWith({ kind: "add_linear_mcp", executor: "codex" });
  expect(within(codex).getByText("codex mcp list")).toBeInTheDocument();
  expect(within(codex).getByText("codex mcp login <name>")).toBeInTheDocument();
});

test("the one-click login says how long it waits, and the page checks again once it is over", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 0, { linear_mcp: false, problems: ["Linear MCP in claude: Needs authentication"] });
  });
  let finish: (() => void) | undefined;
  backend.setupAction = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
  const { client } = await renderWithProviders(<Doctor />, { backend });
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  // The wait is said before the click: the login opens the browser and then waits for the user.
  expect(await within(claude).findByText("Opens your browser. IssueRelay waits up to 5 minutes for you to finish.")).toBeInTheDocument();
  const login = () => within(claude).getByRole("button", { name: "Authorize Linear in Claude Code" });
  await userEvent.click(login());
  // One fix at a time in a card, and no second click while this one waits.
  await waitFor(() => expect(login()).toBeDisabled());
  expect(backend.setupAction).toHaveBeenCalledTimes(1);
  expect(backend.doctor).toHaveBeenCalledTimes(1);

  finish?.();
  await waitFor(() => expect(backend.doctor).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(client.isFetching()).toBe(0));
  expect(backend.doctor).toHaveBeenCalledTimes(2);
  await waitFor(() => expect(login()).toBeEnabled());
});

test("a fix that fails is told with its output, and the button is there to try again", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 0, { linear_mcp: false, problems: ["Linear MCP in claude: Needs authentication"] });
  });
  backend.setupAction = vi.fn().mockRejectedValue(new BackendError("`claude mcp login linear-server` failed: the browser was closed"));
  await renderWithProviders(<Doctor />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" }));
  expect(await screen.findByText("The setup command failed")).toBeInTheDocument();
  expect(screen.getByText("`claude mcp login linear-server` failed: the browser was closed")).toBeInTheDocument();
  await waitFor(() => expect(screen.getByRole("button", { name: "Authorize Linear in Claude Code" })).toBeEnabled());
});

test("a fix is followed by exactly one more check", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const report = await backend.doctor();
  changeExecutor(report, 0, { linear_mcp: false, problems: ["Linear MCP in claude: Needs authentication"] });
  const doctor = vi.spyOn(backend, "doctor").mockResolvedValueOnce(report);
  const { client } = await renderWithProviders(<Doctor />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" }));
  await waitFor(() => expect(doctor).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(client.isFetching()).toBe(0));
  expect(doctor).toHaveBeenCalledTimes(2);
  // The check after the fix found Linear connected: the button is gone.
  await waitFor(() => expect(screen.queryByRole("button", { name: "Authorize Linear in Claude Code" })).not.toBeInTheDocument());
});

test("when it cannot be told how Linear is registered the manual commands are shown, not a guess", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 0, { linear_mcp: false, problems: ["Linear MCP in claude: Needs authentication"] });
  });
  backend.linearServerNamed = vi.fn().mockRejectedValue(new BackendError("`claude mcp get linear-server` was ended by a signal"));
  await renderWithProviders(<Doctor />, { backend });
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  expect(await within(claude).findByText("Could not check how Linear is registered")).toBeInTheDocument();
  expect(within(claude).getByText("`claude mcp get linear-server` was ended by a signal")).toBeInTheDocument();
  expect(within(claude).getByText("claude mcp list")).toBeInTheDocument();
  expect(within(claude).queryByRole("button", { name: /Linear/ })).not.toBeInTheDocument();
  await userEvent.click(within(claude).getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(backend.linearServerNamed).toHaveBeenCalledTimes(2));
});

test("while it is not known how Linear is registered no fix is offered", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 0, { linear_mcp: false, problems: ["Linear MCP in claude: Needs authentication"] });
  });
  backend.linearServerNamed = vi.fn(() => new Promise<boolean>(() => {}));
  await renderWithProviders(<Doctor />, { backend });
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  expect(await within(claude).findByText("Linear MCP is not connected")).toBeInTheDocument();
  expect(within(claude).getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(within(claude).queryByRole("button")).not.toBeInTheDocument();
});

// ---- the runtime environment ------------------------------------------------------------------

test("the environment names where PATH and the proxy were read from, and why it matters", async () => {
  await renderWithProviders(<Doctor />);
  const env = await screen.findByRole("region", { name: "Runtime environment" });
  expect(await within(env).findByText("Your login shell")).toBeInTheDocument();
  expect(within(env).getByText("The system settings")).toBeInTheDocument();
  expect(within(env).getByText("HTTPS_PROXY=http://127.0.0.1:7890")).toBeInTheDocument();
  expect(within(env).getByText(/reads proxy settings only from environment variables/)).toBeInTheDocument();
  expect(within(env).queryByText("Reading the environment had problems")).not.toBeInTheDocument();
});

test("a proxy that carries a password is shown without it, whatever the engine sent", async () => {
  const backend = await demoWith((report) => {
    report.env = {
      path: "/usr/bin",
      path_source: "host",
      proxy: [
        ["HTTPS_PROXY", "http://jane:s3cret@proxy.example:8080"],
        ["NO_PROXY", "localhost,127.0.0.1"],
      ],
      proxy_source: "login-shell",
      problems: [],
    };
  });
  await renderWithProviders(<Doctor />, { backend });
  const env = await screen.findByRole("region", { name: "Runtime environment" });
  expect(await within(env).findByText("HTTPS_PROXY=http://***@proxy.example:8080")).toBeInTheDocument();
  expect(within(env).getByText("NO_PROXY=localhost,127.0.0.1")).toBeInTheDocument();
  expect(document.body).not.toHaveTextContent("s3cret");
});

test("without a proxy the environment says so", async () => {
  const backend = await demoWith((report) => {
    report.env = { path: null, path_source: "host", proxy: [], proxy_source: "none", problems: [] };
  });
  await renderWithProviders(<Doctor />, { backend });
  const env = await screen.findByRole("region", { name: "Runtime environment" });
  expect(await within(env).findByText("No proxy is set")).toBeInTheDocument();
  expect(within(env).getByText("The environment IssueRelay was started with")).toBeInTheDocument();
});

test("what went wrong reading the environment is a detail line under a translated headline", async () => {
  const backend = await demoWith((report) => {
    report.env = {
      path: null,
      path_source: "host",
      proxy: [],
      proxy_source: "none",
      problems: ["the login shell did not answer within 10 seconds"],
    };
  });
  await renderWithProviders(<Doctor />, { backend });
  const env = await screen.findByRole("region", { name: "Runtime environment" });
  expect(await within(env).findByText("Reading the environment had problems")).toBeInTheDocument();
  expect(within(env).getByText("the login shell did not answer within 10 seconds")).toBeInTheDocument();
});

test("an environment that was not resolved is said to be the one IssueRelay started with", async () => {
  const backend = await demoWith((report) => {
    report.env = null;
  });
  await renderWithProviders(<Doctor />, { backend });
  const env = await screen.findByRole("region", { name: "Runtime environment" });
  expect(await within(env).findByText(/get the environment IssueRelay was started with/)).toBeInTheDocument();
});

// ---- the bindings -----------------------------------------------------------------------------

test("each binding says whether its folder exists and its executor is ready", async () => {
  await renderWithProviders(<Doctor />);
  const bindings = await screen.findByRole("region", { name: "Bindings" });
  const project = await within(bindings).findByRole("region", { name: "Demo Project" });
  expect(within(project).getByText("The folder exists")).toBeInTheDocument();
  expect(within(project).getByText("Claude Code is ready")).toBeInTheDocument();
  const mobile = within(bindings).getByRole("region", { name: "Demo Mobile" });
  expect(within(mobile).getByText("The folder exists")).toBeInTheDocument();
  expect(within(mobile).getByText("Codex is not ready")).toBeInTheDocument();
});

test("a check that passed has a green symbol, one that failed a red one", async () => {
  await renderWithProviders(<Doctor />);
  const project = await screen.findByRole("region", { name: "Demo Project" });
  const mobile = screen.getByRole("region", { name: "Demo Mobile" });
  expect(symbolOf(project, "The folder exists")).toContain("text-state-done");
  expect(symbolOf(project, "Claude Code is ready")).toContain("text-state-done");
  expect(symbolOf(mobile, "Codex is not ready")).toContain("text-state-failed");
  const codex = screen.getByRole("region", { name: "Codex" });
  expect(symbolOf(codex, "Installed")).toContain("text-state-done");
  expect(symbolOf(codex, "Not logged in")).toContain("text-state-failed");
  expect(symbolOf(codex, "Linear MCP is connected")).toContain("text-state-done");
});

test("a folder that is gone is said so", async () => {
  const backend = await demoWith((report) => {
    report.bindings[0] = { ...report.bindings[0], directory_exists: false };
  });
  await renderWithProviders(<Doctor />, { backend });
  const project = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(project).getByText("The folder does not exist")).toBeInTheDocument();
  expect(symbolOf(project, "The folder does not exist")).toContain("text-state-failed");
});

test("no bindings yet is an empty state that leads to the bindings", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await renderWithProviders(
    <Routes>
      <Route path="/" element={<Doctor />} />
      <Route path="/bindings" element={<p>the bindings</p>} />
    </Routes>,
    { backend },
  );
  const bindings = await screen.findByRole("region", { name: "Bindings" });
  expect(await within(bindings).findByText("No bindings yet")).toBeInTheDocument();
  await userEvent.click(within(bindings).getByRole("link", { name: "Open the bindings" }));
  expect(await screen.findByText("the bindings")).toBeInTheDocument();
});

// ---- languages --------------------------------------------------------------------------------

test("the page reads in Chinese too, with the engine's lines untranslated beneath", async () => {
  await renderWithProviders(<Doctor />, { lang: "zh-CN" });
  expect(await screen.findByRole("region", { name: "执行者" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "重新走一遍引导" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "重新检查" })).toBeInTheDocument();
  expect(screen.getByText("codex is not logged in")).toBeInTheDocument();
});

// ---- lean runs --------------------------------------------------------------------------------

test("an executor ready for lean runs says so in a part of its own", async () => {
  await renderWithProviders(<Doctor />);
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  const lean = await within(claude).findByRole("region", { name: "Lean runs" });
  expect(within(lean).getByText("Ready")).toBeInTheDocument();
  expect(symbolOf(lean, "Ready")).toContain("text-state-done");
});

test("an executor not ready for lean runs has a translated title with the engine's lines below", async () => {
  const backend = await demoWith((report) => {
    changeExecutor(report, 0, {
      lean_linear_mcp: false,
      lean_problems: ["lean runs use the Linear MCP server named `linear-server` in claude", "p\u0007q"],
    });
  });
  await renderWithProviders(<Doctor />, { backend });
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  const lean = await within(claude).findByRole("region", { name: "Lean runs" });
  expect(within(lean).getByText("Not ready")).toBeInTheDocument();
  expect(symbolOf(lean, "Not ready")).toContain("text-state-failed");
  expect(within(lean).getByText("lean runs use the Linear MCP server named `linear-server` in claude")).toBeInTheDocument();
  // The engine's lines are cleaned like the others.
  expect(within(lean).getByText("p q")).toBeInTheDocument();
  // The usual checks above still pass.
  expect(within(claude).getByText("Linear MCP is connected")).toBeInTheDocument();
});

test("an executor that is not logged in is not ready for lean runs either", async () => {
  await renderWithProviders(<Doctor />);
  const codex = await screen.findByRole("region", { name: "Codex" });
  const lean = await within(codex).findByRole("region", { name: "Lean runs" });
  expect(within(lean).getByText("Not ready")).toBeInTheDocument();
});

import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import App from "@/App";
import { LinearStatus } from "@/components/LinearStatus";
import { queryKeys } from "@/hooks/queries";
import { initI18nOnce, setLanguage } from "@/i18n";
import ja from "@/i18n/locales/ja.json";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { DEMO_SHELL_LINE } from "@/lib/mock-fixtures";
import type { AppInfo, CliPathStatus, ControlServers, Settings as SettingsData } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import Settings from "./Settings";

test("settings save what was changed and keep the rest", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await userEvent.clear(interval);
  await userEvent.type(interval, "60");
  await userEvent.click(screen.getByRole("switch", { name: "Run failed" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        poll_interval_secs: 60,
        comment_language: "follow-ui",
        notifications: expect.objectContaining({ failed: false, run_started: true }),
      }),
    ),
  );
  expect(screen.getByRole("combobox", { name: "AI comment language" })).toHaveTextContent("Follow the interface language");
});

test("values the engine would refuse block saving", async () => {
  await renderWithProviders(<Settings />);
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await userEvent.clear(interval);
  await userEvent.type(interval, "2");
  expect(screen.getByText(/at least 5/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("autostart switches at once and the MCP control server can be registered", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const autostart = vi.spyOn(backend, "setAutostart");
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Start at login" }));
  expect(autostart).toHaveBeenCalledWith(true);
  await userEvent.click(screen.getByRole("button", { name: "Register with Claude Code" }));
  expect(setup).toHaveBeenCalledWith({ kind: "register_control_server", executor: "claude" });
});

test("a registration that could not be checked says so and can be checked again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const check = vi.spyOn(backend, "controlServers").mockResolvedValueOnce({ claude: "unknown", codex: "not_registered" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not check")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Check again" }));
  await waitFor(() => expect(check).toHaveBeenCalledTimes(2));
  expect(await screen.findByRole("button", { name: "Register with Claude Code" })).toBeInTheDocument();
});

test("a rejected API key says so and changes nothing", async () => {
  await renderWithProviders(<Settings />);
  await userEvent.type(await screen.findByLabelText("Personal API key"), "lin_api_bad");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  expect(await screen.findByText("Could not use the API key")).toBeInTheDocument();
  expect(screen.getByText(/did not accept the API key/)).toBeInTheDocument();
});

test("the control server note says unattended runs load it too", async () => {
  await renderWithProviders(<Settings />);
  expect(await screen.findByText(/unattended runs load it too/)).toBeInTheDocument();
});

test("when the settings cannot be read the page says so", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn().mockRejectedValue(new BackendError("cannot read config.toml"));
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not read the settings")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});

// ---- helpers ----------------------------------------------------------------------------------

/** Opens one of the form's lists, by the name of its field, and picks an entry. */
async function pick(field: string, entry: string) {
  await userEvent.click(await screen.findByRole("combobox", { name: field }));
  await userEvent.click(await screen.findByRole("option", { name: entry }));
}

/** The demo backend whose settings are the demo's with `change` applied. */
async function backendWith(change: Partial<SettingsData>) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const settings = await backend.settings();
  backend.settings = vi.fn().mockResolvedValue({ ...settings, ...change });
  return backend;
}

/** The settings the last save was asked to write. */
const lastSaved = (save: ReturnType<typeof vi.spyOn>): SettingsData => save.mock.lastCall![0] as SettingsData;

/** Clears a field and types `text` into it. */
async function retype(field: HTMLElement, text: string) {
  await userEvent.clear(field);
  if (text !== "") await userEvent.type(field, text);
}

// ---- the page ---------------------------------------------------------------------------------

test("the page is eight groups, each named by its heading, in the order of the design", async () => {
  await renderWithProviders(<Settings />);
  expect(await screen.findByRole("heading", { level: 1, name: "Settings" })).toBeInTheDocument();
  const names = ["General", "Monitoring", "Defaults", "Prompt", "Notifications", "System", "MCP control server", "Advanced"];
  await screen.findByRole("region", { name: "Advanced" });
  expect(screen.getAllByRole("heading", { level: 2 }).map((heading) => heading.textContent)).toEqual(names);
  for (const name of names) expect(screen.getByRole("region", { name })).toBeInTheDocument();
});

test("the fields show what is saved", async () => {
  const backend = await backendWith({
    poll_interval_secs: 45,
    max_concurrent: 3,
    run_timeout_mins: 90,
    stall_timeout_mins: 20,
    log_retention_days: 14,
    log_max_total_mb: 512,
    default_executor: "codex",
    default_permission: "personal",
    prompt_template: "Push to the test branch.",
    auto_update: false,
    notifications: { run_started: false, awaiting_acceptance: true, needs_info: true, failed: true },
  });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByRole("spinbutton", { name: "Check every (seconds)" })).toHaveValue(45);
  expect(screen.getByRole("spinbutton", { name: "Runs at the same time (at most)" })).toHaveValue(3);
  expect(screen.getByRole("spinbutton", { name: "Stop a run after (minutes)" })).toHaveValue(90);
  expect(screen.getByRole("spinbutton", { name: "Stuck after no output for (minutes)" })).toHaveValue(20);
  expect(screen.getByRole("spinbutton", { name: "Keep run logs for (days)" })).toHaveValue(14);
  expect(screen.getByRole("spinbutton", { name: "Run logs may take at most (MB)" })).toHaveValue(512);
  expect(screen.getByRole("combobox", { name: "Default executor" })).toHaveTextContent("Codex");
  expect(screen.getByRole("combobox", { name: "Default permission level" })).toHaveTextContent("Personal settings");
  expect(screen.getByRole("textbox", { name: "Default prompt template" })).toHaveValue("Push to the test branch.");
  expect(screen.getByRole("switch", { name: "Check for updates automatically" })).not.toBeChecked();
  expect(screen.getByRole("switch", { name: "Run started" })).not.toBeChecked();
  expect(screen.getByRole("switch", { name: "Ready for acceptance" })).toBeChecked();
  expect(screen.getByRole("textbox", { name: "Trigger label" })).toHaveValue("ai-ready");
  expect(screen.getByRole("textbox", { name: "Status that triggers a run" })).toHaveValue("Todo");
});

test("a page that was not touched has nothing to save but can still be saved", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  expect(screen.queryByText("You have unsaved changes")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save)).toEqual(await backend.settings());
});

test("a change says it is not saved yet, and a save says it was", async () => {
  await renderWithProviders(<Settings />);
  await userEvent.click(await screen.findByRole("switch", { name: "Run failed" }));
  expect(screen.getByText("You have unsaved changes")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("switch", { name: "Run failed" }));
  // Back where it was: nothing to save.
  expect(screen.queryByText("You have unsaved changes")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("switch", { name: "Run failed" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  await waitFor(() => expect(screen.queryByText("You have unsaved changes")).not.toBeInTheDocument());
  expect(screen.getByRole("switch", { name: "Run failed" })).not.toBeChecked();
});

test("the Save button of the page stays in a bar of its own that sticks to the foot of the window", async () => {
  await renderWithProviders(<Settings />);
  const save = await screen.findByRole("button", { name: "Save" });
  const bar = save.closest("[data-slot=save-bar]");
  expect(bar).not.toBeNull();
  expect(bar).toHaveClass("sticky", "bottom-0");
});

test("a save in progress cannot be repeated, and its end is told", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let finish: (() => void) | undefined;
  backend.saveSettings = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Run failed" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toBeDisabled());
  expect(backend.saveSettings).toHaveBeenCalledTimes(1);
  finish?.();
  expect(await screen.findByText("Saved")).toBeInTheDocument();
});

test("a save that fails says why, in the engine's words, and keeps what was typed", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.saveSettings = vi.fn().mockRejectedValue(new BackendError("config.toml was changed while IssueRelay was running"));
  await renderWithProviders(<Settings />, { backend });
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await retype(interval, "90");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Could not save the settings")).toBeInTheDocument();
  expect(screen.getByText("config.toml was changed while IssueRelay was running")).toBeInTheDocument();
  expect(screen.queryByText("Saved")).not.toBeInTheDocument();
  expect(screen.getByRole("spinbutton", { name: "Check every (seconds)" })).toHaveValue(90);
  expect(screen.getByText("You have unsaved changes")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
});

test("what changed elsewhere in the meantime is not written back over", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  const { client } = await renderWithProviders(<Settings />, { backend });
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await retype(interval, "60");
  // Monitoring is paused from the overview while the form is open.
  await backend.pause();
  await client.invalidateQueries({ queryKey: queryKeys.settings });
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save)).toMatchObject({ paused: true, poll_interval_secs: 60 });
});

test("a value typed meanwhile survives a refresh of the saved settings", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const { client } = await renderWithProviders(<Settings />, { backend });
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await retype(interval, "77");
  await backend.pause();
  await client.invalidateQueries({ queryKey: queryKeys.settings });
  await waitFor(() => expect(client.isFetching()).toBe(0));
  expect(screen.getByRole("spinbutton", { name: "Check every (seconds)" })).toHaveValue(77);
});

test("a later read of the settings that fails leaves the form, with what was typed in it", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const { client } = await renderWithProviders(<Settings />, { backend });
  await retype(await screen.findByRole("spinbutton", { name: "Check every (seconds)" }), "77");
  backend.settings = vi.fn().mockRejectedValue(new BackendError("config.toml is being written"));
  await client.invalidateQueries({ queryKey: queryKeys.settings });
  await waitFor(() => expect(client.getQueryState(queryKeys.settings)?.status).toBe("error"));
  expect(screen.queryByText("Could not read the settings")).not.toBeInTheDocument();
  expect(screen.getByRole("spinbutton", { name: "Check every (seconds)" })).toHaveValue(77);
});

// ---- the fields -------------------------------------------------------------------------------

test("the interface language is saved as a code, or as system", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByRole("combobox", { name: "Interface language" })).toHaveTextContent("Follow the system");
  await pick("Interface language", "日本語");
  expect(screen.getByRole("combobox", { name: "Interface language" })).toHaveTextContent("日本語");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save).language).toBe("ja");
  await pick("Interface language", "Follow the system");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(lastSaved(save).language).toBe("system");
});

test("saving another interface language switches the whole panel at once", async () => {
  // The whole panel, not the page alone: the language is followed from the saved settings.
  await initI18nOnce("en");
  window.location.hash = "#/settings";
  try {
    render(<App backend={createMockBackend("demo", { latencyMs: 0 })} />);
    await pick("Interface language", "日本語");
    // Choosing is not saving: the panel is still in English.
    expect(screen.getByRole("link", { name: "Runs" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("link", { name: ja.nav.runs })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: ja.nav.settings })).toBeInTheDocument();
    expect(document.documentElement.lang).toBe("ja");
  } finally {
    window.location.hash = "";
    await setLanguage("en");
  }
});

test("all five languages are offered by their own names, for the interface and for the AI's comments", async () => {
  await renderWithProviders(<Settings />);
  await userEvent.click(await screen.findByRole("combobox", { name: "Interface language" }));
  const names = (await screen.findAllByRole("option")).map((option) => option.textContent);
  expect(names).toEqual(["Follow the system", "简体中文", "繁體中文", "English", "日本語", "한국어"]);
  await userEvent.keyboard("{Escape}");
  await userEvent.click(screen.getByRole("combobox", { name: "AI comment language" }));
  const comment = (await screen.findAllByRole("option")).map((option) => option.textContent);
  expect(comment).toEqual(["Follow the interface language", "简体中文", "繁體中文", "English", "日本語", "한국어"]);
});

test("the language of the AI's comments is saved as a code", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await pick("AI comment language", "한국어");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save).comment_language).toBe("ko");
});

test("a language the list does not have is shown as it is and kept when saving", async () => {
  const backend = await backendWith({ language: "fr", comment_language: "de" });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByRole("combobox", { name: "Interface language" })).toHaveTextContent("fr");
  expect(screen.getByRole("combobox", { name: "AI comment language" })).toHaveTextContent("de");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save)).toMatchObject({ language: "fr", comment_language: "de" });
});

test("the numbers are saved as numbers", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await retype(await screen.findByRole("spinbutton", { name: "Check every (seconds)" }), "45");
  await retype(screen.getByRole("spinbutton", { name: "Runs at the same time (at most)" }), "4");
  await retype(screen.getByRole("spinbutton", { name: "Stop a run after (minutes)" }), "90");
  await retype(screen.getByRole("spinbutton", { name: "Stuck after no output for (minutes)" }), "20");
  await retype(screen.getByRole("spinbutton", { name: "Keep run logs for (days)" }), "14");
  await retype(screen.getByRole("spinbutton", { name: "Run logs may take at most (MB)" }), "512");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save)).toMatchObject({
    poll_interval_secs: 45,
    max_concurrent: 4,
    run_timeout_mins: 90,
    stall_timeout_mins: 20,
    log_retention_days: 14,
    log_max_total_mb: 512,
  });
});

test.each([
  ["Check every (seconds)", "4", /at least 5/],
  ["Runs at the same time (at most)", "0", /at least 1/],
  ["Stop a run after (minutes)", "0", /at least 1/],
  ["Stuck after no output for (minutes)", "0", /at least 1/],
  ["Keep run logs for (days)", "0", /at least 1/],
  ["Run logs may take at most (MB)", "0", /at least 1/],
])("%s below its limit is said to be, and blocks saving", async (field, text, message) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  const input = await screen.findByRole("spinbutton", { name: field });
  await retype(input, text);
  expect(screen.getByText(message)).toBeInTheDocument();
  expect(input).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  expect(save).not.toHaveBeenCalled();
});

test("an emptied number field is below the limit, and a fraction is not a whole number", async () => {
  await renderWithProviders(<Settings />);
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await userEvent.clear(interval);
  expect(screen.getByText("Must be at least 5")).toBeInTheDocument();
  // Set at once: typing a fraction key by key passes through "7.", which a number field does not hold.
  fireEvent.change(interval, { target: { value: "7.5" } });
  expect(screen.getByText("Enter a whole number")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await retype(interval, "8");
  expect(screen.queryByText(/Enter a whole number|at least/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
});

test("a field that is right again lets the page be saved", async () => {
  await renderWithProviders(<Settings />);
  const interval = await screen.findByRole("spinbutton", { name: "Check every (seconds)" });
  await retype(interval, "2");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await retype(interval, "5");
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  expect(interval).toHaveAttribute("aria-invalid", "false");
});

test("the executor and the permission level that new bindings start from are saved", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await pick("Default executor", "Codex");
  await pick("Default permission level", "Automatic review");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save)).toMatchObject({ default_executor: "codex", default_permission: "auto" });
});

test("the permission level says what it means, for the one that is chosen", async () => {
  const backend = await backendWith({ default_permission: "personal" });
  await renderWithProviders(<Settings />, { backend });
  const field = await screen.findByRole("combobox", { name: "Default permission level" });
  expect(field).toHaveAccessibleDescription(/Claude only does what your own settings allow in advance/);
  await pick("Default permission level", "Automatic review");
  expect(field).toHaveAccessibleDescription(/reviews each step by itself/);
});

test("the prompt template is multi-line, and what it is for is said", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  const template = await screen.findByRole("textbox", { name: "Default prompt template" });
  expect(template.tagName).toBe("TEXTAREA");
  expect(template).toHaveAccessibleDescription(/The fixed rules always apply/);
  await userEvent.type(template, "Push to the test branch.{Enter}Read the deployment back.");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save).prompt_template).toBe("Push to the test branch.\nRead the deployment back.");
});

test.each([
  ["Run started", "run_started"],
  ["Ready for acceptance", "awaiting_acceptance"],
  ["Needs information", "needs_info"],
  ["Run failed", "failed"],
] as const)("the notification %s is its own switch", async (name, key) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  const toggle = await screen.findByRole("switch", { name });
  expect(toggle).toBeChecked();
  await userEvent.click(toggle);
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save).notifications).toEqual({ run_started: true, awaiting_acceptance: true, needs_info: true, failed: true, [key]: false });
});

test("automatic updates are saved with the page, and say they take effect later", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  const toggle = await screen.findByRole("switch", { name: "Check for updates automatically" });
  expect(toggle).toBeChecked();
  expect(toggle).toHaveAccessibleDescription("Updates arrive in a later version; your choice is kept until then.");
  await userEvent.click(toggle);
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save).auto_update).toBe(false);
});

// ---- the names in Advanced --------------------------------------------------------------------

test("the label and the statuses are saved, without the blanks around them", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await retype(await screen.findByRole("textbox", { name: "Trigger label" }), "  ready-for-ai ");
  await retype(screen.getByRole("textbox", { name: "Status that triggers a run" }), "Backlog");
  await retype(screen.getByRole("textbox", { name: "Status while the AI works" }), "Doing");
  await retype(screen.getByRole("textbox", { name: "Status when ready for testing" }), "In Review");
  await retype(screen.getByRole("textbox", { name: "Status when accepted" }), "Shipped");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save).trigger).toEqual({
    label: "ready-for-ai",
    todo: "Backlog",
    in_progress: "Doing",
    in_testing: "In Review",
    done: "Shipped",
  });
  // What the form shows afterwards is what was saved: the blanks are gone, and nothing is left to save.
  await waitFor(() => expect(screen.queryByText("You have unsaved changes")).not.toBeInTheDocument());
  expect(screen.getByRole("textbox", { name: "Trigger label" })).toHaveValue("ready-for-ai");
});

test.each([
  ["Trigger label"],
  ["Status that triggers a run"],
  ["Status while the AI works"],
  ["Status when ready for testing"],
  ["Status when accepted"],
])("%s cannot be empty", async (field) => {
  await renderWithProviders(<Settings />);
  const input = await screen.findByRole("textbox", { name: field });
  await userEvent.clear(input);
  expect(screen.getByText("Use a name on one line, without quotes")).toBeInTheDocument();
  expect(input).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("a name with a quote in it is refused, as the engine refuses it", async () => {
  await renderWithProviders(<Settings />);
  const input = await screen.findByRole("textbox", { name: "Status when accepted" });
  await userEvent.type(input, '"');
  expect(screen.getByText("Use a name on one line, without quotes")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("only the name that is wrong says so", async () => {
  await renderWithProviders(<Settings />);
  await userEvent.clear(await screen.findByRole("textbox", { name: "Trigger label" }));
  expect(screen.getAllByText("Use a name on one line, without quotes")).toHaveLength(1);
});

// ---- starting at login ------------------------------------------------------------------------

test("starting at login shows the system's state, and a change shows at once", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.autostartEnabled = vi.fn().mockResolvedValue(true);
  let finish: (() => void) | undefined;
  backend.setAutostart = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
  await renderWithProviders(<Settings />, { backend });
  const toggle = await screen.findByRole("switch", { name: "Start at login" });
  expect(toggle).toBeChecked();
  await userEvent.click(toggle);
  expect(backend.setAutostart).toHaveBeenCalledWith(false);
  // Shown as asked while the system is being told; not saved with the page.
  await waitFor(() => expect(screen.getByRole("switch", { name: "Start at login" })).not.toBeChecked());
  expect(screen.getByRole("switch", { name: "Start at login" })).toBeDisabled();
  finish?.();
  await waitFor(() => expect(screen.getByRole("switch", { name: "Start at login" })).toBeEnabled());
  expect(screen.queryByText("You have unsaved changes")).not.toBeInTheDocument();
});

test("a change of starting at login that the system refused is told and shows the real state again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.setAutostart = vi.fn().mockRejectedValue(new BackendError("the login items could not be changed"));
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Start at login" }));
  expect(await screen.findByText("Could not change start at login")).toBeInTheDocument();
  expect(screen.getByText("the login items could not be changed")).toBeInTheDocument();
  await waitFor(() => expect(screen.getByRole("switch", { name: "Start at login" })).not.toBeChecked());
  expect(screen.getByRole("switch", { name: "Start at login" })).toBeEnabled();
});

test("when it cannot be read whether it starts at login the row says so, with a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.autostartEnabled = vi.fn().mockRejectedValue(new BackendError("the login items could not be read"));
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not read whether IssueRelay starts at login")).toBeInTheDocument();
  expect(screen.getByText("the login items could not be read")).toBeInTheDocument();
  expect(screen.queryByRole("switch", { name: "Start at login" })).not.toBeInTheDocument();
  const system = screen.getByRole("region", { name: "System" });
  await userEvent.click(within(system).getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(backend.autostartEnabled).toHaveBeenCalledTimes(2));
});

// ---- the command line on PATH ----------------------------------------------------------------

/** The clipboard of the test: what is written to it. Removed again after each test. */
function stubClipboard(writeText = vi.fn().mockResolvedValue(undefined)) {
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  return writeText;
}
afterEach(() => {
  Reflect.deleteProperty(navigator, "clipboard");
});

test("the command line goes on PATH from the system group and the shell line is shown", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const install = vi.spyOn(backend, "installCliOnPath");
  await renderWithProviders(<Settings />, { backend });
  // Only there once the state is read: before that the row is a loading row.
  const toggle = await screen.findByRole("switch", { name: "Command line on PATH" });
  expect(toggle).not.toBeChecked();
  expect(toggle).toBeEnabled();
  await userEvent.click(toggle);
  expect(install).toHaveBeenCalledTimes(1);
  expect(await screen.findByText(DEMO_SHELL_LINE)).toBeInTheDocument();
  expect(screen.getByText(/Add this line to your shell profile/)).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeChecked();
});

test("a file IssueRelay did not make is named, cleaned, and does not lock the switch", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = async (): Promise<CliPathStatus> => ({ kind: "blocked", path: "~/.local/bin/issuerelay\u001b[31m" });
  await renderWithProviders(<Settings />, { backend });
  const note = await screen.findByText(/already exists and was not made by IssueRelay/);
  expect(note.textContent).toContain("~/.local/bin/issuerelay");
  // The system's text goes through cleanDisplayText (AI_CODING_GUIDE 12.4).
  expect(note.textContent).not.toContain("\u001b");
  // Shown off, and there to be tried again once the user has removed or renamed the file.
  const toggle = screen.getByRole("switch", { name: "Command line on PATH" });
  expect(toggle).toBeEnabled();
  expect(toggle).not.toBeChecked();
});

test("once the file in the way is gone, turning the switch on installs", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  // What the command line finds in the slot: the file is there until the user has removed it.
  let status: CliPathStatus = { kind: "blocked", path: "~/.local/bin/issuerelay" };
  backend.cliOnPath = async () => status;
  const install = vi.fn(async (): Promise<CliPathStatus> => {
    // The command looks at the slot again: the file is gone, so it links.
    status = { kind: "on", shell_line: DEMO_SHELL_LINE };
    return status;
  });
  backend.installCliOnPath = install;
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText(/already exists and was not made by IssueRelay/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("switch", { name: "Command line on PATH" }));
  expect(install).toHaveBeenCalledTimes(1);
  // The row follows what the command found.
  expect(await screen.findByText(DEMO_SHELL_LINE)).toBeInTheDocument();
  expect(screen.queryByText(/already exists and was not made by IssueRelay/)).not.toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeChecked();
});

test("while the file in the way is still there, turning the switch on says so again and tells no failure", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const blocked: CliPathStatus = { kind: "blocked", path: "~/.local/bin/issuerelay" };
  backend.cliOnPath = vi.fn(async () => blocked);
  const install = vi.fn(async () => blocked);
  backend.installCliOnPath = install;
  await renderWithProviders(<Settings />, { backend });
  await screen.findByText(/already exists and was not made by IssueRelay/);
  await userEvent.click(screen.getByRole("switch", { name: "Command line on PATH" }));
  expect(install).toHaveBeenCalledTimes(1);
  // The row reads the slot again and says the same, and the switch is there to be tried again.
  await waitFor(() => expect(backend.cliOnPath).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeEnabled());
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).not.toBeChecked();
  expect(screen.getByText(/already exists and was not made by IssueRelay/)).toBeInTheDocument();
  // Still in the way is an answer, not a failure of the command.
  expect(screen.queryByText("Could not change the command line on PATH")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("switch", { name: "Command line on PATH" }));
  expect(install).toHaveBeenCalledTimes(2);
});

test("from the disk image the switch explains what to do first", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = async (): Promise<CliPathStatus> => ({ kind: "unavailable", reason: "not_installed" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText(/Move it to Applications first/)).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeDisabled();
});

test("when the PATH setting cannot be read the row says so, with a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = vi.fn().mockRejectedValue(new BackendError("cannot read the link"));
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not read the PATH setting")).toBeInTheDocument();
  expect(screen.getByText("cannot read the link")).toBeInTheDocument();
  expect(screen.queryByRole("switch", { name: "Command line on PATH" })).not.toBeInTheDocument();
  const system = screen.getByRole("region", { name: "System" });
  await userEvent.click(within(system).getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(backend.cliOnPath).toHaveBeenCalledTimes(2));
});

test("the switch takes it off again, and it is not part of Save", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const remove = vi.spyOn(backend, "removeCliFromPath");
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Command line on PATH" }));
  expect(await screen.findByText(DEMO_SHELL_LINE)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("switch", { name: "Command line on PATH" }));
  expect(remove).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(screen.queryByText(DEMO_SHELL_LINE)).not.toBeInTheDocument());
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).not.toBeChecked();
  expect(screen.queryByText("You have unsaved changes")).not.toBeInTheDocument();
});

test("the explanation sets the command it makes available as code", async () => {
  await renderWithProviders(<Settings />);
  await screen.findByRole("switch", { name: "Command line on PATH" });
  expect(screen.getByText("issuerelay", { selector: "code" })).toBeInTheDocument();
  expect(screen.getByText(/in a terminal. No administrator rights needed./)).toBeInTheDocument();
});

test("while the PATH setting is read the row is a loading row, and the switch comes with the answer", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let answer: ((status: CliPathStatus) => void) | undefined;
  backend.cliOnPath = vi.fn(() => new Promise<CliPathStatus>((resolve) => (answer = resolve)));
  await renderWithProviders(<Settings />, { backend });
  // The row above it is there once it has been read; the PATH row is still waiting.
  await screen.findByRole("switch", { name: "Start at login" });
  const system = screen.getByRole("region", { name: "System" });
  expect(within(system).getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(within(system).queryByRole("switch", { name: "Command line on PATH" })).not.toBeInTheDocument();
  answer?.({ kind: "off" });
  expect(await within(system).findByRole("switch", { name: "Command line on PATH" })).toBeEnabled();
  expect(within(system).queryByRole("status", { name: "Loading…" })).not.toBeInTheDocument();
});

test("the switch is described by what the row says about its state, and only a place the app cannot work from locks it", async () => {
  const explanation = "Lets you type issuerelay in a terminal. No administrator rights needed.";
  const cases: [CliPathStatus, RegExp | null][] = [
    [{ kind: "off" }, null],
    [{ kind: "on", shell_line: DEMO_SHELL_LINE }, /Add this line to your shell profile \(for zsh, ~\/\.zshrc\), then open a new terminal:$/],
    [{ kind: "on", shell_line: null }, /On\. Open a new terminal to use it\.$/],
    [{ kind: "stale", target: "/tmp/demo-home/Downloads/IssueRelay.app/Contents/MacOS/issuerelay" }, /points at another copy of IssueRelay/],
    [{ kind: "blocked", path: "~/.local/bin/issuerelay" }, /already exists and was not made by IssueRelay/],
    [{ kind: "unavailable", reason: "not_installed" }, /Move it to Applications first/],
    [{ kind: "unavailable", reason: "development" }, /Not available in a development build/],
    [{ kind: "unavailable", reason: "missing_cli" }, /command-line program is missing from this copy of IssueRelay/],
  ];
  for (const [status, words] of cases) {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.cliOnPath = async () => status;
    const { unmount } = await renderWithProviders(<Settings />, { backend });
    const toggle = await screen.findByRole("switch", { name: "Command line on PATH" });
    // Only a place the app cannot work from locks the switch: a file in the way, a link to another
    // copy, being on PATH already all leave it to be used (the sentence tells what turning it does).
    if (status.kind === "unavailable") expect(toggle, status.kind).toBeDisabled();
    else expect(toggle, status.kind).toBeEnabled();
    // The explanation of the switch itself is always part of it; the state's sentence follows it.
    expect(toggle).toHaveAccessibleDescription(words === null ? explanation : new RegExp(`^${explanation.replace(/[.]/g, "\\.")} .*`));
    if (words !== null) expect(toggle, status.kind).toHaveAccessibleDescription(words);
    unmount();
  }
});

test("the shell line is copied from its own button", async () => {
  const writeText = stubClipboard();
  await renderWithProviders(<Settings />);
  await userEvent.click(await screen.findByRole("switch", { name: "Command line on PATH" }));
  const system = screen.getByRole("region", { name: "System" });
  expect(await within(system).findByText(DEMO_SHELL_LINE)).toBeInTheDocument();
  await userEvent.click(within(system).getByRole("button", { name: "Copy" }));
  expect(writeText).toHaveBeenCalledWith(DEMO_SHELL_LINE);
  expect(await screen.findByText("Copied")).toBeInTheDocument();
});

test("when the shell already finds the folder only a new terminal is needed", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.cliOnPath = async (): Promise<CliPathStatus> => ({ kind: "on", shell_line: null });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("On. Open a new terminal to use it.")).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeChecked();
  expect(screen.queryByText(/Add this line to your shell profile/)).not.toBeInTheDocument();
});

test("a link to another copy is named, cleaned, and the switch points it here", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const target = "/tmp/demo-home/Downloads/IssueRelay.app/Contents/MacOS/issuerelay";
  vi.spyOn(backend, "cliOnPath").mockResolvedValueOnce({ kind: "stale", target: `${target}\u001b[31m` });
  const install = vi.spyOn(backend, "installCliOnPath");
  await renderWithProviders(<Settings />, { backend });
  const note = await screen.findByText(/points at another copy of IssueRelay/);
  expect(note.textContent).toContain(target);
  expect(note.textContent).not.toContain("\u001b");
  const toggle = screen.getByRole("switch", { name: "Command line on PATH" });
  expect(toggle).not.toBeChecked();
  expect(toggle).toBeEnabled();
  await userEvent.click(toggle);
  expect(install).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(screen.queryByText(/points at another copy of IssueRelay/)).not.toBeInTheDocument());
  expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeChecked();
});

test("a change that fails is told with its reason and leaves the switch as it was", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.installCliOnPath = vi.fn().mockRejectedValue(new BackendError("cannot create the link"));
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Command line on PATH" }));
  expect(await screen.findByText("Could not change the command line on PATH")).toBeInTheDocument();
  expect(screen.getByText("cannot create the link")).toBeInTheDocument();
  const toggle = screen.getByRole("switch", { name: "Command line on PATH" });
  expect(toggle).not.toBeChecked();
  expect(toggle).toBeEnabled();
});

test("a development build and a copy without the command-line program say why the switch is off limits", async () => {
  const cases: [CliPathStatus, RegExp][] = [
    [{ kind: "unavailable", reason: "development" }, /Not available in a development build/],
    [{ kind: "unavailable", reason: "missing_cli" }, /command-line program is missing from this copy of IssueRelay/],
  ];
  for (const [status, words] of cases) {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.cliOnPath = async () => status;
    const { unmount } = await renderWithProviders(<Settings />, { backend });
    expect(await screen.findByText(words)).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Command line on PATH" })).toBeDisabled();
    unmount();
  }
});

// ---- full access ------------------------------------------------------------------------------

test("choosing full access for the first time asks, and what the user confirms is saved with the page", async () => {
  const backend = await backendWith({ default_permission: "auto", full_access_acknowledged: false });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await pick("Default permission level", "Full access");
  expect(await screen.findByText("Full access runs without limits")).toBeInTheDocument();
  expect(save).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "I understand" }));
  await waitFor(() => expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument());
  expect(screen.getByRole("combobox", { name: "Default permission level" })).toHaveTextContent("Full access");
  // Nothing is written until the page is saved.
  expect(save).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save)).toMatchObject({ default_permission: "full", full_access_acknowledged: true });
});

test("saying no to full access leaves the level as it was, and nothing is acknowledged", async () => {
  const backend = await backendWith({ default_permission: "auto", full_access_acknowledged: false });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await pick("Default permission level", "Full access");
  await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument());
  expect(screen.getByRole("combobox", { name: "Default permission level" })).toHaveTextContent("Automatic review");
  expect(screen.queryByText("You have unsaved changes")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save)).toMatchObject({ default_permission: "auto", full_access_acknowledged: false });
});

test("Escape says no to full access too", async () => {
  const backend = await backendWith({ default_permission: "personal", full_access_acknowledged: false });
  await renderWithProviders(<Settings />, { backend });
  await pick("Default permission level", "Full access");
  await screen.findByText("Full access runs without limits");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument());
  expect(screen.getByRole("combobox", { name: "Default permission level" })).toHaveTextContent("Personal settings");
});

test("saying no to full access returns the focus to the permission list, where the user was", async () => {
  const backend = await backendWith({ default_permission: "auto", full_access_acknowledged: false });
  await renderWithProviders(<Settings />, { backend });
  await pick("Default permission level", "Full access");
  await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument());
  // The page is some 2,200 px tall: with the focus on the body a keyboard user would be back at its top.
  await waitFor(() => expect(screen.getByRole("combobox", { name: "Default permission level" })).toHaveFocus());
});

test("understanding full access returns the focus to the permission list as well", async () => {
  const backend = await backendWith({ default_permission: "auto", full_access_acknowledged: false });
  await renderWithProviders(<Settings />, { backend });
  await pick("Default permission level", "Full access");
  await userEvent.click(await screen.findByRole("button", { name: "I understand" }));
  await waitFor(() => expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument());
  await waitFor(() => expect(screen.getByRole("combobox", { name: "Default permission level" })).toHaveFocus());
});

test("full access that was understood before is not asked about again", async () => {
  const backend = await backendWith({ default_permission: "auto", full_access_acknowledged: true });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await pick("Default permission level", "Full access");
  expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "Default permission level" })).toHaveTextContent("Full access");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(lastSaved(save)).toMatchObject({ default_permission: "full", full_access_acknowledged: true });
});

test("a lower level needs no question, and the question comes once per page", async () => {
  const backend = await backendWith({ default_permission: "full", full_access_acknowledged: false });
  await renderWithProviders(<Settings />, { backend });
  await pick("Default permission level", "Personal settings");
  expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument();
  await pick("Default permission level", "Full access");
  await userEvent.click(await screen.findByRole("button", { name: "I understand" }));
  await pick("Default permission level", "Automatic review");
  await pick("Default permission level", "Full access");
  expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument();
});

test("the question is not asked when full access is what is saved already", async () => {
  const backend = await backendWith({ default_permission: "full", full_access_acknowledged: false });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByRole("combobox", { name: "Default permission level" })).toHaveTextContent("Full access");
  expect(screen.queryByText("Full access runs without limits")).not.toBeInTheDocument();
});

// ---- the MCP control server -------------------------------------------------------------------

/** The row of one executor in the MCP control server group. */
const serverRow = (executor: string) => within(screen.getByRole("region", { name: "MCP control server" })).getByRole("listitem", { name: executor });

test("each executor has a row that says whether the control server is registered", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  vi.spyOn(backend, "controlServers").mockResolvedValue({ claude: "registered", codex: "not_registered" });
  await renderWithProviders(<Settings />, { backend });
  await screen.findByText("Registered");
  expect(within(serverRow("Claude Code")).getByText("Registered")).toBeInTheDocument();
  expect(within(serverRow("Claude Code")).getByRole("button", { name: "Remove from Claude Code" })).toBeEnabled();
  expect(within(serverRow("Codex")).getByText("Not registered")).toBeInTheDocument();
  expect(within(serverRow("Codex")).getByRole("button", { name: "Register with Codex" })).toBeEnabled();
});

test("removing the control server sends the one command and the row follows", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  vi.spyOn(backend, "controlServers").mockResolvedValueOnce({ claude: "registered", codex: "not_registered" });
  const setup = vi.spyOn(backend, "setupAction");
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Remove from Claude Code" }));
  expect(setup).toHaveBeenCalledWith({ kind: "remove_control_server", executor: "claude" });
  expect(await within(serverRow("Claude Code")).findByRole("button", { name: "Register with Claude Code" })).toBeInTheDocument();
});

test("registering shows the row registered once it is", async () => {
  await renderWithProviders(<Settings />);
  await userEvent.click(await screen.findByRole("button", { name: "Register with Codex" }));
  expect(await within(serverRow("Codex")).findByRole("button", { name: "Remove from Codex" })).toBeInTheDocument();
  expect(within(serverRow("Codex")).getByText("Registered")).toBeInTheDocument();
});

test("an executor that is not installed has nothing to register with", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  vi.spyOn(backend, "controlServers").mockResolvedValue({ claude: "registered", codex: "not_installed" });
  await renderWithProviders(<Settings />, { backend });
  expect(await within(await screen.findByRole("region", { name: "MCP control server" })).findByText("Not installed")).toBeInTheDocument();
  expect(within(serverRow("Codex")).queryByRole("button")).not.toBeInTheDocument();
});

test("a registration that fails is told with the command's own words, and the button is there again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.setupAction = vi.fn().mockRejectedValue(new BackendError("`claude mcp add` failed: permission denied"));
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Register with Claude Code" }));
  expect(await screen.findByText("The setup command failed")).toBeInTheDocument();
  expect(screen.getByText("`claude mcp add` failed: permission denied")).toBeInTheDocument();
  await waitFor(() => expect(screen.getByRole("button", { name: "Register with Claude Code" })).toBeEnabled());
});

test("while one registration runs its own button waits", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let finish: (() => void) | undefined;
  backend.setupAction = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Register with Claude Code" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Register with Claude Code" })).toBeDisabled());
  expect(screen.getByRole("button", { name: "Register with Codex" })).toBeEnabled();
  finish?.();
  await waitFor(() => expect(screen.getByRole("button", { name: "Register with Claude Code" })).toBeEnabled());
});

test("without IssueRelay's command-line program nothing can be registered, and the page says what to do", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const info: AppInfo = await backend.appInfo();
  backend.appInfo = vi.fn().mockResolvedValue({ ...info, cli_path: null });
  vi.spyOn(backend, "controlServers").mockResolvedValue({ claude: "not_registered", codex: "registered" });
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText(/command-line program was not found/)).toBeInTheDocument();
  expect(screen.getByText("cargo build -p issuerelay-cli").tagName).toBe("CODE");
  expect(screen.getByRole("button", { name: "Register with Claude Code" })).toBeDisabled();
  // Removing needs no program of ours.
  expect(screen.getByRole("button", { name: "Remove from Codex" })).toBeEnabled();
});

test("with the command-line program found there is no such note", async () => {
  await renderWithProviders(<Settings />);
  await screen.findByRole("button", { name: "Register with Claude Code" });
  expect(screen.queryByText(/command-line program was not found/)).not.toBeInTheDocument();
});

test("when the registrations cannot be read at all the group says so, with a retry", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const answer = backend.controlServers;
  backend.controlServers = vi.fn(answer).mockRejectedValueOnce(new BackendError("IssueRelay could not start"));
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not check the MCP control server")).toBeInTheDocument();
  expect(screen.getByText("IssueRelay could not start")).toBeInTheDocument();
  const group = screen.getByRole("region", { name: "MCP control server" });
  await userEvent.click(within(group).getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("button", { name: "Register with Claude Code" })).toBeInTheDocument();
});

test("both registrations that could not be checked can be checked again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const unknown: ControlServers = { claude: "unknown", codex: "unknown" };
  const check = vi.spyOn(backend, "controlServers").mockResolvedValueOnce(unknown);
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findAllByText("Could not check")).toHaveLength(2);
  await userEvent.click(within(serverRow("Codex")).getByRole("button", { name: "Check again" }));
  await waitFor(() => expect(check).toHaveBeenCalledTimes(2));
  expect(await screen.findByRole("button", { name: "Register with Codex" })).toBeInTheDocument();
});

test("the note says it is registered for the user, that unattended runs load it, and that its tools can pause", async () => {
  await renderWithProviders(<Settings />);
  const note = await screen.findByText(/unattended runs load it too/);
  expect(note).toHaveTextContent("It is registered for your user, so unattended runs load it too; its tools can pause monitoring.");
  expect(screen.getAllByText(/unattended runs load it too/)).toHaveLength(1);
});

// ---- the API key ------------------------------------------------------------------------------

test("the API key is in Advanced, in a password field", async () => {
  await renderWithProviders(<Settings />);
  const advanced = await screen.findByRole("region", { name: "Advanced" });
  expect(within(advanced).getByLabelText("Personal API key")).toHaveAttribute("type", "password");
  expect(within(advanced).getByRole("button", { name: "Use this key" })).toBeDisabled();
});

test("while a browser login waits the key cannot be used, and the page says why", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  backend.linearLogin = vi.fn(() => new Promise<never>(() => {}));
  await renderWithProviders(
    <>
      <LinearStatus />
      <Settings />
    </>,
    { backend },
  );
  await userEvent.type(await screen.findByLabelText("Personal API key"), "lin_api_demo");
  await userEvent.click(screen.getByRole("button", { name: "Log in to Linear" }));
  expect(await screen.findByText("Finish or cancel the browser login first")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Use this key" })).toBeDisabled();
});

test("a key that was accepted is not shown again, and the page says the key is in use", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const use = vi.spyOn(backend, "linearUseApiKey");
  await renderWithProviders(<Settings />, { backend });
  await userEvent.type(await screen.findByLabelText("Personal API key"), "lin_api_demo");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  expect(await screen.findByText("Using the API key now")).toBeInTheDocument();
  expect(use).toHaveBeenCalledWith("lin_api_demo");
  expect(screen.getByLabelText("Personal API key")).toHaveValue("");
  expect(document.body).not.toHaveTextContent("lin_api_demo");
});

test("using a key does not save the page", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const save = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Settings />, { backend });
  await userEvent.click(await screen.findByRole("switch", { name: "Run failed" }));
  await userEvent.type(screen.getByLabelText("Personal API key"), "lin_api_demo");
  await userEvent.type(screen.getByLabelText("Personal API key"), "{Enter}");
  expect(await screen.findByText("Using the API key now")).toBeInTheDocument();
  expect(save).not.toHaveBeenCalled();
  // What was changed on the page is still waiting to be saved.
  expect(screen.getByText("You have unsaved changes")).toBeInTheDocument();
});

// ---- the states -------------------------------------------------------------------------------

test("while the settings are read the page is a loading state", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const settings = await backend.settings();
  let release: ((value: SettingsData) => void) | undefined;
  backend.settings = vi.fn(() => new Promise<SettingsData>((resolve) => (release = resolve)));
  await renderWithProviders(<Settings />, { backend });
  expect(screen.getByRole("heading", { level: 1, name: "Settings" })).toBeInTheDocument();
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
  release?.(settings);
  expect(await screen.findByRole("button", { name: "Save" })).toBeInTheDocument();
});

test("settings that cannot be read are told in the engine's words under the headline, and can be read again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const answer = backend.settings;
  backend.settings = vi.fn(answer).mockRejectedValueOnce(new BackendError("cannot read config.toml"));
  await renderWithProviders(<Settings />, { backend });
  expect(await screen.findByText("Could not read the settings")).toBeInTheDocument();
  expect(screen.getByText("cannot read config.toml")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("button", { name: "Save" })).toBeInTheDocument();
});

test("the page reads in Chinese, and the language names stay in their own language", async () => {
  await renderWithProviders(<Settings />, { lang: "zh-CN" });
  expect(await screen.findByRole("region", { name: "常规" })).toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "界面语言" })).toHaveTextContent("跟随系统");
  await userEvent.click(screen.getByRole("combobox", { name: "界面语言" }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual(["跟随系统", "简体中文", "繁體中文", "English", "日本語", "한국어"]);
});

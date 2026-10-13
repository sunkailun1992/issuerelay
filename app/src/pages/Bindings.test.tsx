import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useLocation } from "react-router";
import { vi } from "vitest";
import { queryKeys } from "@/hooks/queries";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { DEMO_BINDINGS, DEMO_DOCTOR } from "@/lib/mock-fixtures";
import type { Binding, DoctorReport, Settings } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import Bindings from "./Bindings";

test("bindings list their folder, executor and permission", async () => {
  await renderWithProviders(<Bindings />);
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(demo).getByText("~/code/demo-app")).toBeInTheDocument();
  expect(within(demo).getByText("Full access")).toBeInTheDocument();
  expect(within(demo).getByText("Default (Claude Code)")).toBeInTheDocument();
  const mobile = screen.getByRole("region", { name: "Demo Mobile" });
  expect(within(mobile).getByText("Automatic review")).toBeInTheDocument();
  expect(within(mobile).getByText("gpt-5-codex")).toBeInTheDocument();
});

test("removing a binding asks first", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const remove = vi.spyOn(backend, "removeBinding");
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Remove" }));
  expect(remove).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(remove).toHaveBeenCalledWith("proj-1");
});

test("when the bindings cannot be read the page says so", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.bindings = vi.fn().mockRejectedValue(new BackendError("config.toml was changed while IssueRelay was running"));
  await renderWithProviders(<Bindings />, { backend });
  expect(await screen.findByText("Could not read the bindings")).toBeInTheDocument();
  expect(screen.getByText("config.toml was changed while IssueRelay was running")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
});

test("no bindings yet invites the first one", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await backend.linearLogin();
  await renderWithProviders(<Bindings />, { backend });
  expect(await screen.findByRole("button", { name: "Add your first binding" })).toBeInTheDocument();
});

/** Opens one of the form's lists, by the name of its field, and picks an entry. */
async function pick(field: string, entry: string) {
  await userEvent.click(await screen.findByRole("combobox", { name: field }));
  await userEvent.click(await screen.findByRole("option", { name: entry }));
}

/** Shows where the router is, so that a test can follow a button. */
function Where() {
  const location = useLocation();
  return <p data-testid="where">{location.pathname}</p>;
}

/** The names of the cards on the page, in order (each card has its project's name as a heading). */
const cardNames = () => screen.getAllByRole("heading", { level: 2 }).map((heading) => heading.textContent);

/** The demo backend whose bindings are `bindings`. */
function backendWithBindings(bindings: Binding[]) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.bindings = vi.fn().mockResolvedValue(bindings);
  return backend;
}

test("each binding is a section named for its project, in the order of the configuration", async () => {
  await renderWithProviders(<Bindings />);
  await screen.findByRole("region", { name: "Demo Project" });
  expect(screen.getByRole("heading", { level: 1, name: "Bindings" })).toBeInTheDocument();
  expect(cardNames()).toEqual(["Demo Project", "Demo Mobile"]);
  expect(within(screen.getByRole("region", { name: "Demo Mobile" })).getByRole("heading", { level: 2, name: "Demo Mobile" })).toBeInTheDocument();
});

test("a card has the executor's own setting where the binding names no model, and no prompt override", async () => {
  await renderWithProviders(<Bindings />);
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(demo).getByText("The executor's own setting")).toBeInTheDocument();
  expect(within(demo).getByText("None")).toBeInTheDocument();
  expect(within(demo).queryByRole("button", { name: /Prompt override/ })).not.toBeInTheDocument();
});

test("a binding that follows the settings says what they are", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.bindings = vi.fn().mockResolvedValue([{ id: "proj-1", project_id: "proj-1", project_name: "Demo Project", directory: "~/code/demo-app" }]);
  const settings = await backend.settings();
  backend.settings = async () => ({ ...settings, default_executor: "codex", default_permission: "personal" });
  await renderWithProviders(<Bindings />, { backend });
  const card = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(card).getByText("Default (Codex)")).toBeInTheDocument();
  expect(within(card).getByText("Default (Personal settings)")).toBeInTheDocument();
});

test("when the settings cannot be read the defaults are said to be unknown, not guessed", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.bindings = vi.fn().mockResolvedValue([{ id: "proj-1", project_id: "proj-1", project_name: "Demo Project", directory: "~/code/demo-app" }]);
  backend.settings = vi.fn().mockRejectedValue(new BackendError("settings are gone"));
  await renderWithProviders(<Bindings />, { backend });
  const card = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(card).getAllByText("Default (Unknown)")).toHaveLength(2);
});

test("the cards wait for the settings, so that they never say 'Unknown' about what is only not read yet", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const settings = await backend.settings();
  let release: ((value: Settings) => void) | undefined;
  backend.settings = vi.fn(() => new Promise<Settings>((resolve) => (release = resolve)));
  const { client } = await renderWithProviders(<Bindings />, { backend });
  // The bindings are read, the settings are not: nothing is shown but the loading state.
  await waitFor(() => expect(client.getQueryState(queryKeys.bindings)?.status).toBe("success"));
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { level: 2 })).not.toBeInTheDocument();
  release?.(settings);
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(demo).getByText("Default (Claude Code)")).toBeInTheDocument();
  expect(screen.queryByText(/Unknown/)).not.toBeInTheDocument();
});

test("the page shows that it is loading until the bindings are read", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.bindings = vi.fn(() => new Promise<Binding[]>(() => {}));
  await renderWithProviders(<Bindings />, { backend });
  expect(await screen.findByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { level: 1, name: "Bindings" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Add binding" })).not.toBeInTheDocument();
});

test("trying again after a failed read reads the bindings again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.bindings = vi.fn().mockRejectedValueOnce(new BackendError("config.toml is locked")).mockResolvedValue(DEMO_BINDINGS);
  await renderWithProviders(<Bindings />, { backend });
  await screen.findByText("Could not read the bindings");
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("region", { name: "Demo Project" })).toBeInTheDocument();
  expect(screen.queryByText("Could not read the bindings")).not.toBeInTheDocument();
});

test("the button to add a binding is at the top right of the page", async () => {
  await renderWithProviders(<Bindings />);
  await screen.findByRole("region", { name: "Demo Project" });
  const header = screen.getByRole("heading", { level: 1, name: "Bindings" }).closest("header");
  expect(header).not.toBeNull();
  expect(within(header as HTMLElement).getByRole("button", { name: "Add binding" })).toBeInTheDocument();
});

test("the empty page explains what a binding is and opens the dialog", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await backend.linearLogin();
  await renderWithProviders(<Bindings />, { backend });
  expect(await screen.findByText("No bindings yet")).toBeInTheDocument();
  expect(screen.getByText(/only works on issues of a project that is bound/)).toBeInTheDocument();
  // The empty state has its own button; the header has none then.
  expect(screen.queryByRole("button", { name: "Add binding" })).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Add your first binding" }));
  expect(await screen.findByRole("dialog", { name: "Add a binding" })).toBeInTheDocument();
});

test("a binding is added in a dialog, which closes when it is saved, and the new card appears", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await renderWithProviders(<Bindings />, { backend });
  await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(screen.getByRole("button", { name: "Add binding" }));
  const dialog = await screen.findByRole("dialog", { name: "Add a binding" });
  await pick("Linear project", "Docs Site");
  await userEvent.click(within(dialog).getByRole("button", { name: "Choose…" }));
  await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  const docs = await screen.findByRole("region", { name: "Docs Site" });
  expect(within(docs).getByText("~/code/new-project")).toBeInTheDocument();
  expect(cardNames()).toEqual(["Demo Project", "Demo Mobile", "Docs Site"]);
});

test("the first binding with full access asks first, inside the dialog, and both are gone once it is saved", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await backend.linearLogin();
  const saveSettings = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<Bindings />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Add your first binding" }));
  const dialog = await screen.findByRole("dialog", { name: "Add a binding" });
  await pick("Linear project", "Demo Project");
  await userEvent.click(within(dialog).getByRole("button", { name: "Choose…" }));
  await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
  const warning = await screen.findByRole("alertdialog", { name: "Full access runs without limits" });
  // Saying no leaves the form where it was.
  await userEvent.click(within(warning).getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  expect(screen.getByRole("dialog", { name: "Add a binding" })).toBeInTheDocument();
  expect(saveSettings).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await userEvent.click(await screen.findByRole("button", { name: "I understand" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(await screen.findByRole("region", { name: "Demo Project" })).toBeInTheDocument();
  expect(saveSettings).toHaveBeenCalledWith(expect.objectContaining({ full_access_acknowledged: true }));
  // It was asked once: the next binding with full access goes straight in.
  await userEvent.click(screen.getByRole("button", { name: "Add binding" }));
  await pick("Linear project", "Docs Site");
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  expect(cardNames()).toEqual(["Demo Project", "Docs Site"]);
});

test("cancelling the dialog adds nothing", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  await renderWithProviders(<Bindings />, { backend });
  await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(screen.getByRole("button", { name: "Add binding" }));
  const dialog = await screen.findByRole("dialog");
  await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(saveBinding).not.toHaveBeenCalled();
  expect(cardNames()).toEqual(["Demo Project", "Demo Mobile"]);
});

test("a binding is edited in a dialog that starts with what it has, and the card shows the change", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Edit" }));
  const dialog = await screen.findByRole("dialog", { name: "Edit the binding" });
  expect(await within(dialog).findByRole("textbox", { name: "Folder" })).toHaveValue("~/code/demo-app");
  expect(within(dialog).getByRole("combobox", { name: "Permission level" })).toHaveTextContent("Full access");
  await pick("Executor", "Codex");
  await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  const edited = await screen.findByRole("region", { name: "Demo Project" });
  await waitFor(() => expect(within(edited).getByText("Codex")).toBeInTheDocument());
  expect(within(edited).queryByText("Default (Claude Code)")).not.toBeInTheDocument();
});

test("a dialog opened for another binding does not keep what the last one had", async () => {
  await renderWithProviders(<Bindings />);
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Edit" }));
  expect(await screen.findByRole("textbox", { name: "Linear project" })).toHaveValue("Demo Project");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  await userEvent.click(within(screen.getByRole("region", { name: "Demo Mobile" })).getByRole("button", { name: "Edit" }));
  expect(await screen.findByRole("textbox", { name: "Linear project" })).toHaveValue("Demo Mobile");
  expect(screen.getByRole("textbox", { name: "Model" })).toHaveValue("gpt-5-codex");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  await userEvent.click(screen.getByRole("button", { name: "Add binding" }));
  expect(await screen.findByRole("combobox", { name: "Linear project" })).toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Model" })).toHaveValue("");
});

test("cancelling the question keeps the binding", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const remove = vi.spyOn(backend, "removeBinding");
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Remove" }));
  const question = await screen.findByRole("alertdialog", { name: "Remove this binding?" });
  expect(question).toHaveTextContent("Issues of Demo Project will wait until the project is bound again. The folder itself is not touched.");
  await userEvent.click(within(question).getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  expect(remove).not.toHaveBeenCalled();
  expect(screen.getByRole("region", { name: "Demo Project" })).toBeInTheDocument();
});

test("a confirmed removal takes the card away", async () => {
  await renderWithProviders(<Bindings />);
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Remove" }));
  await userEvent.click(await screen.findByRole("button", { name: "Confirm" }));
  await waitFor(() => expect(screen.queryByRole("region", { name: "Demo Project" })).not.toBeInTheDocument());
  expect(screen.getByRole("region", { name: "Demo Mobile" })).toBeInTheDocument();
});

test("removing the last binding leaves the invitation to add one", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await backend.removeBinding("proj-2");
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Remove" }));
  await userEvent.click(await screen.findByRole("button", { name: "Confirm" }));
  expect(await screen.findByRole("button", { name: "Add your first binding" })).toBeInTheDocument();
});

test("a removal that fails is reported with the engine's line below a translated headline", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.removeBinding = vi.fn().mockRejectedValue(new BackendError("config.toml was changed while IssueRelay was running"));
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Remove" }));
  await userEvent.click(await screen.findByRole("button", { name: "Confirm" }));
  expect(await screen.findByText("Could not remove the binding")).toBeInTheDocument();
  expect(screen.getByText("config.toml was changed while IssueRelay was running")).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Demo Project" })).toBeInTheDocument();
});

const LONG_FOLDER = "~/code/an-extremely-long-folder-name/with/several/levels/demo-app";

test("a folder too long for the card is shortened in the middle and shown whole on hover", async () => {
  const backend = backendWithBindings([{ ...DEMO_BINDINGS[0], directory: LONG_FOLDER }]);
  await renderWithProviders(<Bindings />, { backend });
  const card = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(card).getByText(LONG_FOLDER)).toBeInTheDocument();
  expect(within(card).getByTitle(LONG_FOLDER)).toBeInTheDocument();
  // What is seen is the start and the last 24 characters.
  expect(within(card).getByText(LONG_FOLDER.slice(-24))).toHaveAttribute("aria-hidden", "true");
});

test("the folder is shown without escape sequences or invisible characters", async () => {
  const backend = backendWithBindings([{ ...DEMO_BINDINGS[0], directory: `~/code/${String.fromCharCode(27)}[2Jdemo-app${String.fromCharCode(0x202e)}` }]);
  await renderWithProviders(<Bindings />, { backend });
  const card = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(card).getByText("~/code/demo-app")).toBeInTheDocument();
});

test("a prompt override is a closed row that opens to the text", async () => {
  const text = "Push to the test branch.\nDeploy with the script in scripts/deploy.sh.";
  const backend = backendWithBindings([{ ...DEMO_BINDINGS[0], prompt_override: text }]);
  await renderWithProviders(<Bindings />, { backend });
  const card = await screen.findByRole("region", { name: "Demo Project" });
  // Read together with the name of the row it is in.
  const open = within(card).getByRole("button", { name: "Prompt override Yes" });
  expect(open).toHaveAttribute("aria-expanded", "false");
  expect(within(card).queryByText(/Deploy with the script/)).not.toBeInTheDocument();
  await userEvent.click(open);
  expect(open).toHaveAttribute("aria-expanded", "true");
  const shown = within(card).getByText(/Deploy with the script/);
  expect(shown.tagName).toBe("PRE");
  expect(shown.textContent).toBe(text);
  await userEvent.click(open);
  await waitFor(() => expect(within(card).queryByText(/Deploy with the script/)).not.toBeInTheDocument());
});

test("an override with nothing in it counts as none", async () => {
  const backend = backendWithBindings([{ ...DEMO_BINDINGS[0], prompt_override: "  \n " }]);
  await renderWithProviders(<Bindings />, { backend });
  const card = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(card).getByText("None")).toBeInTheDocument();
});

test("the check results come from the last environment check, which the page does not run by itself", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const doctor = vi.spyOn(backend, "doctor");
  const projects = vi.spyOn(backend, "projects");
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  const mobile = screen.getByRole("region", { name: "Demo Mobile" });
  // The check runs the executors and asks Linear: it is not run on a visit, and nor are the projects.
  expect(within(demo).getByText("Not checked yet")).toBeInTheDocument();
  expect(within(mobile).getByText("Not checked yet")).toBeInTheDocument();
  expect(doctor).not.toHaveBeenCalled();
  expect(projects).not.toHaveBeenCalled();
  await userEvent.click(within(demo).getByRole("button", { name: "Check again" }));
  expect(await within(demo).findByText("The folder exists")).toBeInTheDocument();
  expect(within(demo).getByText("Claude Code is ready")).toBeInTheDocument();
  // One check serves every card.
  expect(within(mobile).getByText("The folder exists")).toBeInTheDocument();
  expect(within(mobile).getByText("Codex is not ready")).toBeInTheDocument();
  expect(within(demo).queryByText("Not checked yet")).not.toBeInTheDocument();
  expect(within(mobile).queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
  expect(doctor).toHaveBeenCalledTimes(1);
  // What is found is also in the symbol in front of it (the sentence is what is read out): green for
  // what holds, red for what does not.
  const symbol = (card: HTMLElement, sentence: string) => within(card).getByText(sentence).closest("li")!.querySelector("svg");
  expect(symbol(demo, "The folder exists")).toHaveClass("text-state-done");
  expect(symbol(demo, "Claude Code is ready")).toHaveClass("text-state-done");
  expect(symbol(mobile, "Codex is not ready")).toHaveClass("text-state-failed");
  expect(symbol(mobile, "Codex is not ready")).not.toHaveClass("text-state-done");
  // An executor that is not ready is mended in the environment check, and the card points there; a card with nothing to mend does not.
  expect(within(mobile).getByRole("link", { name: "Open the environment check" })).toHaveAttribute("href", "/doctor");
  expect(within(demo).queryByRole("link", { name: "Open the environment check" })).not.toBeInTheDocument();
});

test("the result of a check is announced to assistive technology as it arrives", async () => {
  await renderWithProviders(<Bindings />);
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  expect(within(demo).getByText("Not checked yet").closest("dd")).toHaveAttribute("aria-live", "polite");
});

test("a check that was run elsewhere is shown at once", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const { client } = await renderWithProviders(<Bindings />, { backend });
  await screen.findByRole("region", { name: "Demo Project" });
  // The environment check page reads the same cached report.
  await client.fetchQuery({ queryKey: queryKeys.doctor, queryFn: () => backend.doctor() });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  expect(await within(demo).findByText("The folder exists")).toBeInTheDocument();
  expect(within(demo).queryByText("Not checked yet")).not.toBeInTheDocument();
});

test("a check in progress is said so", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let finish: (() => void) | undefined;
  backend.doctor = vi.fn(
    () =>
      new Promise<DoctorReport>((resolve) => {
        finish = () => resolve(DEMO_DOCTOR);
      }),
  );
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Check again" }));
  expect(await within(demo).findByText("Checking…")).toBeInTheDocument();
  expect(within(demo).queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
  finish?.();
  expect(await within(demo).findByText("The folder exists")).toBeInTheDocument();
});

test("a check that fails says so and can be run again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.doctor = vi.fn().mockRejectedValueOnce(new BackendError("the environment check timed out")).mockResolvedValue(DEMO_DOCTOR);
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Check again" }));
  expect(await within(demo).findByText("Could not run the check")).toBeInTheDocument();
  expect(within(demo).getByText("the environment check timed out")).toBeInTheDocument();
  await userEvent.click(within(demo).getByRole("button", { name: "Check again" }));
  expect(await within(demo).findByText("The folder exists")).toBeInTheDocument();
  expect(within(demo).queryByText("Could not run the check")).not.toBeInTheDocument();
});

test("a folder that is missing and an executor that is not ready come with the way to the environment check", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.doctor = vi.fn().mockResolvedValue({
    ...DEMO_DOCTOR,
    bindings: [
      { binding_id: "proj-1", project_name: "Demo Project", directory_exists: false, executor: "claude", executor_ready: true },
      { binding_id: "proj-2", project_name: "Demo Mobile", directory_exists: true, executor: "codex", executor_ready: true },
    ],
  });
  await renderWithProviders(
    <>
      <Bindings />
      <Where />
    </>,
    { backend },
  );
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  const mobile = screen.getByRole("region", { name: "Demo Mobile" });
  await userEvent.click(within(demo).getByRole("button", { name: "Check again" }));
  expect(await within(demo).findByText("The folder does not exist")).toBeInTheDocument();
  expect(within(demo).getByText("The folder does not exist").closest("li")!.querySelector("svg")).toHaveClass("text-state-failed");
  expect(within(demo).getByText("Claude Code is ready")).toBeInTheDocument();
  // Nothing to fix, no link.
  expect(within(mobile).queryByRole("link", { name: "Open the environment check" })).not.toBeInTheDocument();
  await userEvent.click(within(demo).getByRole("link", { name: "Open the environment check" }));
  expect(screen.getByTestId("where")).toHaveTextContent("/doctor");
});

test("the check and the removal go by the binding's own id, which need not be its project's", async () => {
  const odd: Binding = { ...DEMO_BINDINGS[0], id: "binding-from-before" };
  const backend = backendWithBindings([odd]);
  backend.doctor = vi.fn().mockResolvedValue({
    ...DEMO_DOCTOR,
    bindings: [{ binding_id: "binding-from-before", project_name: "Demo Project", directory_exists: false, executor: "claude", executor_ready: true }],
  });
  const remove = vi.spyOn(backend, "removeBinding");
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Check again" }));
  expect(await within(demo).findByText("The folder does not exist")).toBeInTheDocument();
  await userEvent.click(within(demo).getByRole("button", { name: "Remove" }));
  await userEvent.click(await screen.findByRole("button", { name: "Confirm" }));
  expect(remove).toHaveBeenCalledWith("binding-from-before");
});

test("a binding the last check did not know is not reported as fine", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.doctor = vi.fn().mockResolvedValue({ ...DEMO_DOCTOR, bindings: [DEMO_DOCTOR.bindings[0]] });
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Check again" }));
  expect(await within(demo).findByText("The folder exists")).toBeInTheDocument();
  const mobile = screen.getByRole("region", { name: "Demo Mobile" });
  expect(within(mobile).getByText("Not checked yet")).toBeInTheDocument();
  expect(within(mobile).queryByText("The folder exists")).not.toBeInTheDocument();
});

test("a change to the bindings makes the last check outdated, and it is not shown as if it still held", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await renderWithProviders(<Bindings />, { backend });
  const demo = await screen.findByRole("region", { name: "Demo Project" });
  await userEvent.click(within(demo).getByRole("button", { name: "Check again" }));
  await within(demo).findByText("The folder exists");
  // Another folder: what was checked is not what is bound any more.
  await userEvent.click(within(demo).getByRole("button", { name: "Edit" }));
  const dialog = await screen.findByRole("dialog");
  await userEvent.click(await within(dialog).findByRole("button", { name: "Choose…" }));
  await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  const edited = await screen.findByRole("region", { name: "Demo Project" });
  await waitFor(() => expect(within(edited).getByText("~/code/new-project")).toBeInTheDocument());
  expect(within(edited).getByText("Not checked yet")).toBeInTheDocument();
  expect(within(screen.getByRole("region", { name: "Demo Mobile" })).getByText("Not checked yet")).toBeInTheDocument();
  // And a new check brings the results back.
  await userEvent.click(within(edited).getByRole("button", { name: "Check again" }));
  expect(await within(edited).findByText("The folder exists")).toBeInTheDocument();
});

import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { queryKeys } from "@/hooks/queries";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { DEMO_BINDINGS, DEMO_PROJECTS } from "@/lib/mock-fixtures";
import type { PermissionProfile, Project, Settings } from "@/lib/types";
import { endFades, holdClosingDialogs } from "@/test/fade";
import { renderWithProviders } from "@/test/render";
import { BindingDialog, BindingForm } from "./BindingDialog";

test("binding a project the first time with full access shows the warning once", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await backend.linearLogin();
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const saveSettings = vi.spyOn(backend, "saveSettings");
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await userEvent.click(await screen.findByRole("combobox", { name: "Linear project" }));
  await userEvent.click(await screen.findByRole("option", { name: "Demo Project" }));
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText(/run any command on this computer/)).toBeInTheDocument();
  expect(saveBinding).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "I understand" }));
  await waitFor(() => expect(saveBinding).toHaveBeenCalledWith(expect.objectContaining({ project_id: "proj-1", directory: "~/code/new-project" })));
  expect(saveSettings).toHaveBeenCalledWith(expect.objectContaining({ full_access_acknowledged: true }));
});

test("an invalid model name blocks saving with a hint", async () => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />);
  // user-event reads "[" as the start of a key name; "[[" types one "[".
  await userEvent.type(await screen.findByRole("textbox", { name: "Model" }), "sonnet[[1m]");
  expect(screen.getByRole("textbox", { name: "Model" })).toHaveValue("sonnet[1m]");
  expect(screen.getByText(/names with brackets/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("a folder the engine refuses is explained at the top of the form", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.chooseDirectory = vi.fn().mockResolvedValue("code/new-project");
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm onSaved={onSaved} />, { backend });
  await userEvent.click(await screen.findByRole("combobox", { name: "Linear project" }));
  await userEvent.click(await screen.findByRole("option", { name: "Docs Site" }));
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const alert = await screen.findByRole("alert");
  expect(within(alert).getByText("Could not save the binding")).toBeInTheDocument();
  expect(within(alert).getByText('bindings[2].directory must be an absolute path: "code/new-project"')).toBeInTheDocument();
  expect(onSaved).not.toHaveBeenCalled();
});

/** A backend whose user has logged in to Linear and has (not) confirmed the full-access warning. */
async function backendWith({ defaultPermission = "full", acknowledged = false }: { defaultPermission?: PermissionProfile; acknowledged?: boolean } = {}) {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  await backend.linearLogin();
  const settings = await backend.settings();
  await backend.saveSettings({ ...settings, default_permission: defaultPermission, full_access_acknowledged: acknowledged });
  return backend;
}

/** Opens one of the form's lists, by the name of its field, and picks an entry. */
async function pick(field: string, entry: string) {
  await userEvent.click(await screen.findByRole("combobox", { name: field }));
  await userEvent.click(await screen.findByRole("option", { name: entry }));
}

/** Chooses a project and a folder: what the form needs before it can be saved. */
async function fillTheEssentials(project = "Docs Site") {
  await pick("Linear project", project);
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

test.each(["project first", "folder first"])("saving waits for a project and a folder (%s)", async (order) => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />);
  const save = await screen.findByRole("button", { name: "Save" });
  expect(save).toBeDisabled();
  expect(screen.getByRole("textbox", { name: "Folder" })).toHaveValue("");
  const chooseProject = () => pick("Linear project", "Docs Site");
  const chooseFolder = () => userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await (order === "project first" ? chooseProject() : chooseFolder());
  // One of the two is not enough.
  expect(save).toBeDisabled();
  await (order === "project first" ? chooseFolder() : chooseProject());
  expect(screen.getByRole("textbox", { name: "Folder" })).toHaveValue("~/code/new-project");
  expect(save).toBeEnabled();
});

test("the folder is only chosen with the picker; closing the picker keeps the folder", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.chooseDirectory = vi.fn().mockResolvedValueOnce("~/code/first").mockResolvedValueOnce(null);
  const saveBinding = vi.spyOn(backend, "saveBinding");
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  const folder = await screen.findByRole("textbox", { name: "Folder" });
  expect(folder).toHaveAttribute("readonly");
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  expect(folder).toHaveValue("~/code/first");
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  expect(backend.chooseDirectory).toHaveBeenCalledTimes(2);
  expect(folder).toHaveValue("~/code/first");
  // What would be saved is the folder as well, not only what the field shows.
  await pick("Linear project", "Docs Site");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
  expect(saveBinding.mock.calls[0][0].directory).toBe("~/code/first");
});

test("a picker that fails is reported and the folder stays as it was", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.chooseDirectory = vi.fn().mockRejectedValue(new BackendError("no window to show the dialog in"));
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Choose…" }));
  expect(await screen.findByText("Could not open the folder picker")).toBeInTheDocument();
  expect(screen.getByText("no window to show the dialog in")).toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Folder" })).toHaveValue("");
});

test("what is chosen is what is saved, and nothing else", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm onSaved={onSaved} />, { backend });
  await fillTheEssentials();
  await pick("Executor", "Codex");
  await pick("Permission level", "Automatic review");
  // A pasted name often has a blank at an end; it is not part of the name.
  await userEvent.type(screen.getByRole("textbox", { name: "Model" }), " gpt-5-codex ");
  await userEvent.type(screen.getByRole("textbox", { name: "Prompt override" }), "Push to the test branch.");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
  expect(saveBinding).toHaveBeenCalledTimes(1);
  expect(saveBinding).toHaveBeenCalledWith({
    id: "proj-3",
    project_id: "proj-3",
    project_name: "Docs Site",
    directory: "~/code/new-project",
    executor: "codex",
    permission: "auto",
    model: "gpt-5-codex",
    prompt_override: "Push to the test branch.",
  });
});

test("what is left at its default is not written into the binding", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  // Blanks only are as good as nothing.
  await userEvent.type(screen.getByRole("textbox", { name: "Model" }), "  ");
  await userEvent.type(screen.getByRole("textbox", { name: "Prompt override" }), "   ");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
  expect(Object.keys(saveBinding.mock.calls[0][0]).sort()).toEqual(["directory", "id", "project_id", "project_name"]);
});

test("the list of projects marks the ones that are bound already and does not let them be chosen", async () => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />);
  await userEvent.click(await screen.findByRole("combobox", { name: "Linear project" }));
  const options = await screen.findAllByRole("option");
  expect(options.map((option) => option.textContent)).toEqual(["Demo Project (already bound)", "Demo Mobile (already bound)", "Docs Site"]);
  const bound = options.filter((option) => option.getAttribute("aria-disabled") === "true");
  expect(bound).toHaveLength(2);
  // Choosing a project that is bound would replace its binding without anyone asking for that.
  await userEvent.click(bound[0]);
  // The list stays open, since nothing was chosen; close it.
  await userEvent.keyboard("{Escape}");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  expect(screen.getByRole("combobox", { name: "Linear project" })).toHaveTextContent("Choose a project");
});

test("when the projects cannot be read the field says so and tries again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.projects = vi.fn().mockRejectedValueOnce(new BackendError("not connected to Linear: log in first")).mockResolvedValue(DEMO_PROJECTS);
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  expect(await screen.findByText("Could not read the projects from Linear")).toBeInTheDocument();
  expect(screen.getByText("not connected to Linear: log in first")).toBeInTheDocument();
  // The rest of the form is there to be filled in meanwhile.
  expect(screen.getByRole("button", { name: "Choose…" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("combobox", { name: "Linear project" })).toBeInTheDocument();
  expect(screen.queryByText("Could not read the projects from Linear")).not.toBeInTheDocument();
});

test("a Linear without projects says there is nothing to bind", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.projects = vi.fn().mockResolvedValue([]);
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  expect(await screen.findByText("Linear has no projects to bind yet. Create one in Linear first.")).toBeInTheDocument();
});

test("the lists that follow the settings say what the default is", async () => {
  const backend = await backendWith({ defaultPermission: "personal", acknowledged: true });
  const settings = await backend.settings();
  backend.settings = async () => ({ ...settings, default_executor: "codex" });
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  expect(await screen.findByRole("combobox", { name: "Executor" })).toHaveTextContent("Default (Codex)");
  expect(screen.getByRole("combobox", { name: "Permission level" })).toHaveTextContent("Default (Personal settings)");
  // And the sentence under the level is the one of the level that the default stands for.
  expect(screen.getByRole("combobox", { name: "Permission level" })).toHaveAccessibleDescription(/must be allowed there, or the run fails/);
  await userEvent.click(screen.getByRole("combobox", { name: "Executor" }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual(["Default (Codex)", "Claude Code", "Codex"]);
});

test("each permission level says what it means, under the field", async () => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />);
  const level = await screen.findByRole("combobox", { name: "Permission level" });
  // "Default" is the settings' level (full access in a new installation), and its description is shown.
  expect(level).toHaveTextContent("Default (Full access)");
  expect(screen.getByText(/never asked to confirm anything/)).toBeInTheDocument();
  expect(level).toHaveAccessibleDescription(/never asked to confirm anything/);
  await pick("Permission level", "Automatic review");
  expect(screen.getByText(/reviews each step by itself/)).toBeInTheDocument();
  expect(screen.queryByText(/never asked to confirm anything/)).not.toBeInTheDocument();
  await pick("Permission level", "Personal settings");
  expect(screen.getByText(/must be allowed there, or the run fails/)).toBeInTheDocument();
  expect(screen.getByText(/Codex follows ~\/\.codex\/config\.toml/)).toBeInTheDocument();
});

test("the permission levels are listed in the order of the design, after the default", async () => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />);
  await userEvent.click(await screen.findByRole("combobox", { name: "Permission level" }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual([
    "Default (Full access)",
    "Full access",
    "Automatic review",
    "Personal settings",
  ]);
});

test("Enter in a field saves the form when it can be saved, and does nothing when it cannot", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  const model = screen.getByRole("textbox", { name: "Model" });
  await userEvent.type(model, "-rf{Enter}");
  expect(saveBinding).not.toHaveBeenCalled();
  await userEvent.clear(model);
  await userEvent.type(model, "sonnet{Enter}");
  await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
  expect(saveBinding.mock.calls[0][0].model).toBe("sonnet");
});

test.each<{ defaultPermission: PermissionProfile; chosen: string | null; acknowledged: boolean; warns: boolean }>([
  // "Default" is the level of the settings, whatever it is.
  { defaultPermission: "full", chosen: null, acknowledged: false, warns: true },
  { defaultPermission: "auto", chosen: null, acknowledged: false, warns: false },
  { defaultPermission: "personal", chosen: null, acknowledged: false, warns: false },
  // An explicit level is that level, whatever the default is.
  { defaultPermission: "auto", chosen: "Full access", acknowledged: false, warns: true },
  { defaultPermission: "full", chosen: "Automatic review", acknowledged: false, warns: false },
  { defaultPermission: "full", chosen: "Personal settings", acknowledged: false, warns: false },
  // Once confirmed, it is not asked again.
  { defaultPermission: "full", chosen: null, acknowledged: true, warns: false },
  { defaultPermission: "auto", chosen: "Full access", acknowledged: true, warns: false },
])(
  "the warning (default $defaultPermission, chosen $chosen, confirmed before: $acknowledged) is shown: $warns",
  async ({ defaultPermission, chosen, acknowledged, warns }) => {
    const backend = await backendWith({ defaultPermission, acknowledged });
    const saveBinding = vi.spyOn(backend, "saveBinding");
    const saveSettings = vi.spyOn(backend, "saveSettings");
    await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
    await fillTheEssentials();
    if (chosen !== null) await pick("Permission level", chosen);
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    if (warns) {
      expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
      expect(saveBinding).not.toHaveBeenCalled();
    } else {
      await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      // Nothing was confirmed, so nothing is written to the settings either.
      expect(saveSettings).toHaveBeenCalledTimes(0);
    }
  },
);

test("confirming the warning saves the settings with the confirmation, the rest untouched, before the binding", async () => {
  const backend = await backendWith();
  const before = await backend.settings();
  const order: string[] = [];
  const saveSettings = vi.spyOn(backend, "saveSettings").mockImplementation(async () => {
    order.push("settings");
  });
  vi.spyOn(backend, "saveBinding").mockImplementation(async () => {
    order.push("binding");
  });
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm onSaved={onSaved} />, { backend });
  await fillTheEssentials();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await userEvent.click(await screen.findByRole("button", { name: "I understand" }));
  await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
  expect(saveSettings).toHaveBeenCalledTimes(1);
  expect(saveSettings).toHaveBeenCalledWith({ ...before, full_access_acknowledged: true });
  expect(order).toEqual(["settings", "binding"]);
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});

test("cancelling the warning saves nothing and leaves the form as it was", async () => {
  const backend = await backendWith();
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const saveSettings = vi.spyOn(backend, "saveSettings");
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm onSaved={onSaved} />, { backend });
  await fillTheEssentials();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  expect(saveBinding).not.toHaveBeenCalled();
  expect(saveSettings).not.toHaveBeenCalled();
  expect(onSaved).not.toHaveBeenCalled();
  // The folder and the project are still there, and a lower level can be chosen instead.
  expect(screen.getByRole("textbox", { name: "Folder" })).toHaveValue("~/code/new-project");
  await pick("Permission level", "Automatic review");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
  expect(saveSettings).not.toHaveBeenCalled();
});

test("when the confirmation cannot be written the binding is not saved either, and the user can try again", async () => {
  const backend = await backendWith();
  // The first writing is refused, the next one goes through.
  const saveSettings = vi.fn().mockRejectedValueOnce(new BackendError("config.toml was changed while IssueRelay was running")).mockResolvedValue(undefined);
  backend.saveSettings = saveSettings;
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm onSaved={onSaved} />, { backend });
  await fillTheEssentials();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await userEvent.click(await screen.findByRole("button", { name: "I understand" }));
  expect(await screen.findByText("Could not save the settings")).toBeInTheDocument();
  expect(screen.getByText("config.toml was changed while IssueRelay was running")).toBeInTheDocument();
  expect(saveBinding).not.toHaveBeenCalled();
  expect(onSaved).not.toHaveBeenCalled();
  // The form is still there to try again, and the warning is asked again: nothing was confirmed.
  await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await userEvent.click(await screen.findByRole("button", { name: "I understand" }));
  await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
  expect(saveSettings).toHaveBeenCalledTimes(2);
  expect(saveBinding).toHaveBeenCalledTimes(1);
});

test("a model name that is fixed lets the form be saved again", async () => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />);
  await fillTheEssentials();
  const model = screen.getByRole("textbox", { name: "Model" });
  expect(screen.getByText(/Leave empty to use the executor's own setting/)).toBeInTheDocument();
  await userEvent.type(model, "-rf");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  expect(model).toBeInvalid();
  expect(model).toHaveAccessibleDescription(/Only letters, digits and/);
  expect(screen.queryByText(/Leave empty to use the executor's own setting/)).not.toBeInTheDocument();
  await userEvent.clear(model);
  await userEvent.type(model, "sonnet");
  expect(model).toBeValid();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  expect(screen.queryByText(/Only letters, digits and/)).not.toBeInTheDocument();
});

test("a binding cannot be saved twice by clicking again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const pending = deferred<void>();
  const saveBinding = vi.fn(() => pending.promise);
  backend.saveBinding = saveBinding;
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm onSaved={onSaved} />, { backend });
  await fillTheEssentials();
  const save = screen.getByRole("button", { name: "Save" });
  await userEvent.click(save);
  await waitFor(() => expect(save).toBeDisabled());
  await userEvent.click(save);
  expect(saveBinding).toHaveBeenCalledTimes(1);
  expect(onSaved).not.toHaveBeenCalled();
  pending.resolve();
  await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
});

test("a failure is reported once, in the form, and goes away with the next change", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.chooseDirectory = vi.fn().mockResolvedValueOnce("code/new-project").mockResolvedValue("~/code/new-project");
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm onSaved={onSaved} />, { backend });
  await fillTheEssentials();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByRole("alert");
  // The form's own message is the only one: no toast says it a second time.
  expect(screen.getAllByText("Could not save the binding")).toHaveLength(1);
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
});

test("a failure to save is never shown as a bare error", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  // An engine line with a terminal escape in it: the form shows it cleaned, below a translated headline.
  backend.saveBinding = vi.fn().mockRejectedValue(new BackendError(`disk is full${String.fromCharCode(27)}[2J`));
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const alert = await screen.findByRole("alert");
  expect(within(alert).getByText("Could not save the binding")).toBeInTheDocument();
  expect(within(alert).getByText("disk is full")).toBeInTheDocument();
});

test("an existing binding keeps its project and shows what it has", async () => {
  const mobile = DEMO_BINDINGS[1];
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const onSaved = vi.fn();
  await renderWithProviders(<BindingForm initial={mobile} onSaved={onSaved} />, { backend });
  // The project is fixed: a binding is for one project, and the list of projects is not even asked for.
  const project = await screen.findByRole("textbox", { name: "Linear project" });
  expect(project).toHaveValue("Demo Mobile");
  expect(project).toHaveAttribute("readonly");
  expect(screen.getByRole("textbox", { name: "Folder" })).toHaveValue("~/code/demo-mobile");
  expect(screen.getByRole("combobox", { name: "Executor" })).toHaveTextContent("Codex");
  expect(screen.getByRole("combobox", { name: "Permission level" })).toHaveTextContent("Automatic review");
  expect(screen.getByRole("textbox", { name: "Model" })).toHaveValue("gpt-5-codex");
  expect(screen.getByRole("textbox", { name: "Prompt override" })).toHaveValue("");
  // It can be saved as it is, and a change is saved under the same id.
  const save = screen.getByRole("button", { name: "Save" });
  expect(save).toBeEnabled();
  await userEvent.clear(screen.getByRole("textbox", { name: "Model" }));
  await userEvent.type(screen.getByRole("textbox", { name: "Model" }), "gpt-5");
  await userEvent.click(save);
  await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
  expect(saveBinding).toHaveBeenCalledWith({ ...mobile, model: "gpt-5" });
});

test("an existing binding is saved under its own id, which need not be its project's", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const odd = { ...DEMO_BINDINGS[1], id: "binding-from-before" };
  await renderWithProviders(<BindingForm initial={odd} onSaved={() => {}} />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Save" }));
  await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
  expect(saveBinding).toHaveBeenCalledWith(odd);
  expect(saveBinding.mock.calls[0][0].id).toBe("binding-from-before");
  expect(saveBinding.mock.calls[0][0].project_id).toBe("proj-2");
});

test("a model or a prompt that is emptied is taken out of the binding", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const withPrompt = { ...DEMO_BINDINGS[1], prompt_override: "Use the staging database." };
  await renderWithProviders(<BindingForm initial={withPrompt} onSaved={() => {}} />, { backend });
  expect(await screen.findByRole("textbox", { name: "Prompt override" })).toHaveValue("Use the staging database.");
  await userEvent.clear(screen.getByRole("textbox", { name: "Model" }));
  await userEvent.clear(screen.getByRole("textbox", { name: "Prompt override" }));
  await pick("Executor", "Default (Claude Code)");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
  expect(saveBinding.mock.calls[0][0]).toEqual({
    id: "proj-2",
    project_id: "proj-2",
    project_name: "Demo Mobile",
    directory: "~/code/demo-mobile",
    permission: "auto",
  });
  expect("executor" in saveBinding.mock.calls[0][0]).toBe(false);
});

test("a prompt override is saved as it was typed, line breaks and blanks at the ends and all", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  await userEvent.type(screen.getByRole("textbox", { name: "Prompt override" }), "  First line{Enter}  Indented second line{Enter}");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
  expect(saveBinding.mock.calls[0][0].prompt_override).toBe("  First line\n  Indented second line\n");
});

test("without readable settings the form cannot say what the defaults are, and offers to try again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn().mockRejectedValueOnce(new BackendError("settings are gone")).mockResolvedValue(await backend.settings());
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  expect(await screen.findByText("Could not read the settings")).toBeInTheDocument();
  expect(screen.getByText("settings are gone")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("combobox", { name: "Executor" })).toBeInTheDocument();
});

test("the cancel button is there when the form is asked for one", async () => {
  const onCancel = vi.fn();
  await renderWithProviders(<BindingForm onSaved={() => {}} onCancel={onCancel} />);
  // The form's own button, not the one of the loading state that it replaces.
  await screen.findByRole("button", { name: "Save" });
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onCancel).toHaveBeenCalledTimes(1);
});

test("without a cancel handler there is no cancel button", async () => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />);
  await screen.findByRole("button", { name: "Save" });
  expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
});

test("the dialog is named for what it does, has no untranslated close button and closes on cancel", async () => {
  const onOpenChange = vi.fn();
  await renderWithProviders(<BindingDialog open onOpenChange={onOpenChange} />);
  const dialog = await screen.findByRole("dialog", { name: "Add a binding" });
  expect(dialog).toHaveAccessibleDescription("Choose the Linear project and the folder the AI works in.");
  // The library's close button is not there (its label is one thing, the dialog has Cancel and Escape): nothing named "Close".
  expect(within(dialog).queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
  await within(dialog).findByRole("button", { name: "Save" });
  await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

test("an existing binding is edited in a dialog of that name, and saving closes it", async () => {
  const onOpenChange = vi.fn();
  await renderWithProviders(<BindingDialog open onOpenChange={onOpenChange} initial={DEMO_BINDINGS[0]} />);
  const dialog = await screen.findByRole("dialog", { name: "Edit the binding" });
  await userEvent.click(await within(dialog).findByRole("button", { name: "Save" }));
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
});

test("the Escape key closes the dialog", async () => {
  const onOpenChange = vi.fn();
  await renderWithProviders(<BindingDialog open onOpenChange={onOpenChange} />);
  await screen.findByRole("dialog");
  await userEvent.keyboard("{Escape}");
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

// Focus.

test.each(["Cancel", "Escape"])("closing the warning with %s returns the focus to the Save button", async (how) => {
  const backend = await backendWith();
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const warning = await screen.findByRole("alertdialog");
  // The warning starts on the safe answer.
  expect(within(warning).getByRole("button", { name: "Cancel" })).toHaveFocus();
  if (how === "Cancel") await userEvent.click(within(warning).getByRole("button", { name: "Cancel" }));
  else await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  // Without it focus falls to the page, and a keyboard user starts again from its top.
  await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toHaveFocus());
});

test("when nothing had the focus where the warning was asked for, it goes back to the Save button", async () => {
  // A button that is clicked does not take the focus in every browser (Safari's, which the macOS
  // panel is shown in, leaves it where it was): then nothing is there to give it back to.
  const backend = await backendWith();
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  (document.activeElement as HTMLElement).blur();
  expect(document.activeElement).toBe(document.body);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByRole("alertdialog");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toHaveFocus());
});

test("a warning that Enter in a field asked for gives the focus back to that field", async () => {
  const backend = await backendWith();
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  const model = screen.getByRole("textbox", { name: "Model" });
  await userEvent.type(model, "sonnet{Enter}");
  await screen.findByRole("alertdialog");
  await userEvent.keyboard("{Escape}");
  // The user was typing there, not at the Save button.
  await waitFor(() => expect(model).toHaveFocus());
});

test("the form of an existing binding starts on its first editable control, not on the project that cannot change", async () => {
  await renderWithProviders(<BindingDialog open onOpenChange={() => {}} initial={DEMO_BINDINGS[1]} />);
  const project = await screen.findByRole("textbox", { name: "Linear project" });
  await waitFor(() => expect(screen.getByRole("button", { name: "Choose…" })).toHaveFocus());
  expect(project).not.toHaveFocus();
});

// The loading and the failing state of the form.

test("while the settings are read the form offers Cancel", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn(() => new Promise<Settings>(() => {}));
  const onCancel = vi.fn();
  await renderWithProviders(<BindingForm onSaved={() => {}} onCancel={onCancel} />, { backend });
  expect(await screen.findByRole("status", { name: "Loading…" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onCancel).toHaveBeenCalledTimes(1);
});

test("when the settings cannot be read the form offers Cancel beside Try again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn().mockRejectedValue(new BackendError("settings are gone"));
  const onCancel = vi.fn();
  await renderWithProviders(<BindingForm onSaved={() => {}} onCancel={onCancel} />, { backend });
  expect(await screen.findByText("Could not read the settings")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onCancel).toHaveBeenCalledTimes(1);
});

test("without a cancel handler the loading and the failing state have no Cancel button", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn().mockRejectedValue(new BackendError("settings are gone"));
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await screen.findByText("Could not read the settings");
  expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
});

test("the dialog can be closed from its failing state", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn().mockRejectedValue(new BackendError("settings are gone"));
  const onOpenChange = vi.fn();
  await renderWithProviders(<BindingDialog open onOpenChange={onOpenChange} />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

// Saving, and what must not happen meanwhile.

test("while a save is on its way Cancel is disabled and neither Escape nor a click outside closes the dialog", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const pending = deferred<void>();
  backend.saveBinding = vi.fn(() => pending.promise);
  const onOpenChange = vi.fn();
  await renderWithProviders(<BindingDialog open onOpenChange={onOpenChange} />, { backend });
  const dialog = await screen.findByRole("dialog");
  await fillTheEssentials();
  await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
  await waitFor(() => expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled());
  await userEvent.keyboard("{Escape}");
  await userEvent.click(document.querySelector("[data-slot=dialog-overlay]")!);
  expect(onOpenChange).not.toHaveBeenCalled();
  pending.resolve();
  // The save is over and the dialog closes by itself.
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
});

test("a save that fails while the dialog was asked to close is not lost: its failure is in the form when it ends", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const pending = deferred<void>();
  backend.saveBinding = vi.fn(() => pending.promise);
  const onOpenChange = vi.fn();
  await renderWithProviders(<BindingDialog open onOpenChange={onOpenChange} />, { backend });
  const dialog = await screen.findByRole("dialog");
  await fillTheEssentials();
  await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
  await waitFor(() => expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled());
  await userEvent.keyboard("{Escape}");
  pending.reject(new BackendError("disk is full"));
  const alert = await screen.findByRole("alert");
  expect(within(alert).getByText("Could not save the binding")).toBeInTheDocument();
  expect(within(alert).getByText("disk is full")).toBeInTheDocument();
  expect(onOpenChange).not.toHaveBeenCalled();
  // Now that nothing is on its way the dialog closes as it always does.
  await waitFor(() => expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeEnabled());
  await userEvent.keyboard("{Escape}");
  expect(onOpenChange).toHaveBeenCalledWith(false);
});

test("while the confirmation of the warning is written the dialog does not close either", async () => {
  const backend = await backendWith();
  const pending = deferred<void>();
  backend.saveSettings = vi.fn(() => pending.promise);
  const onOpenChange = vi.fn();
  await renderWithProviders(<BindingDialog open onOpenChange={onOpenChange} />, { backend });
  const dialog = await screen.findByRole("dialog");
  await fillTheEssentials();
  await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
  await userEvent.click(await screen.findByRole("button", { name: "I understand" }));
  await waitFor(() => expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled());
  await userEvent.keyboard("{Escape}");
  expect(onOpenChange).not.toHaveBeenCalled();
  pending.resolve();
  await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
});

test("'I understand' pressed again while the warning is still fading out answers once", async () => {
  const backend = await backendWith();
  const pending = deferred<void>();
  const saveSettings = vi.fn(() => pending.promise);
  backend.saveSettings = saveSettings;
  const saveBinding = vi.spyOn(backend, "saveBinding");
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  // The lists above close at once; it is the warning that fades.
  holdClosingDialogs();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const understand = await screen.findByRole("button", { name: "I understand" });
  await userEvent.click(understand);
  // The warning is closing but its button is still there, and a second click reaches it.
  expect(understand).toBeInTheDocument();
  await userEvent.click(understand);
  await endFades();
  expect(saveSettings).toHaveBeenCalledTimes(1);
  pending.resolve();
  await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
});

test("'I understand' activated twice before the form has drawn again answers once", async () => {
  const backend = await backendWith();
  const pending = deferred<void>();
  const saveSettings = vi.fn(() => pending.promise);
  backend.saveSettings = saveSettings;
  const saveBinding = vi.spyOn(backend, "saveBinding");
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  holdClosingDialogs();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const understand = await screen.findByRole("button", { name: "I understand" });
  // Two clicks in one go: the form has not drawn between them, so what it knows has not changed.
  act(() => {
    understand.click();
    understand.click();
  });
  await endFades();
  expect(saveSettings).toHaveBeenCalledTimes(1);
  pending.resolve();
  await waitFor(() => expect(saveBinding).toHaveBeenCalledTimes(1));
});

test("a project that is gone from Linear's list by the time of saving is reported, not ignored", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const { client } = await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  await fillTheEssentials();
  // Linear's list is read again meanwhile: the project is no longer in it, and another one is.
  await act(async () => {
    client.setQueryData<Project[]>(queryKeys.projects, [...DEMO_PROJECTS.filter((project) => project.id !== "proj-3"), { id: "proj-4", name: "Blog", team_ids: [] }]);
  });
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("The project you chose is not in the list from Linear any more. Choose one again.");
  expect(saveBinding).not.toHaveBeenCalled();
  // The list shows that nothing is chosen now, and saving waits for a project.
  expect(screen.getByRole("combobox", { name: "Linear project" })).toHaveTextContent("Choose a project");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  // Choosing another project takes the message away.
  await pick("Linear project", "Blog");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
});

// What a label names.

/** Every `for` of a label names an element that is on the page. */
function expectLabelsToNameSomething() {
  const labels = [...document.querySelectorAll("label[for]")];
  expect(labels.length).toBeGreaterThan(0);
  for (const label of labels) {
    expect(document.getElementById(label.getAttribute("for") ?? ""), `the label "${label.textContent}"`).not.toBeNull();
  }
}

test("each label of the form names a control that is there, whatever state the project field is in", async () => {
  // The project field when the list is there...
  const normal = await renderWithProviders(<BindingForm onSaved={() => {}} />);
  await screen.findByRole("combobox", { name: "Linear project" });
  expectLabelsToNameSomething();
  normal.unmount();
  // ...when the list cannot be read...
  const failing = createMockBackend("demo", { latencyMs: 0 });
  failing.projects = vi.fn().mockRejectedValue(new BackendError("not connected to Linear: log in first"));
  const failed = await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend: failing });
  await screen.findByText("Could not read the projects from Linear");
  expectLabelsToNameSomething();
  failed.unmount();
  // ...while it is being read...
  const slow = createMockBackend("demo", { latencyMs: 0 });
  slow.projects = vi.fn(() => new Promise<Project[]>(() => {}));
  const waiting = await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend: slow });
  await screen.findByRole("textbox", { name: "Model" });
  expect(screen.getAllByRole("status", { name: "Loading…" })).toHaveLength(1);
  expectLabelsToNameSomething();
  waiting.unmount();
  // ...when there is none...
  const empty = createMockBackend("demo", { latencyMs: 0 });
  empty.projects = vi.fn().mockResolvedValue([]);
  const none = await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend: empty });
  await screen.findByText("Linear has no projects to bind yet. Create one in Linear first.");
  expectLabelsToNameSomething();
  none.unmount();
  // ...and for a binding that exists.
  await renderWithProviders(<BindingForm initial={DEMO_BINDINGS[0]} onSaved={() => {}} />);
  await screen.findByRole("textbox", { name: "Linear project" });
  expectLabelsToNameSomething();
});

test.each([
  ["en", ["Demo Project (already bound)", "Demo Mobile (already bound)", "Docs Site"], "Linear project"],
  ["zh-CN", ["Demo Project（已绑定）", "Demo Mobile（已绑定）", "Docs Site"], "Linear 项目"],
  ["zh-TW", ["Demo Project（已綁定）", "Demo Mobile（已綁定）", "Docs Site"], "Linear 專案"],
  ["ja", ["Demo Project（紐付け済み）", "Demo Mobile（紐付け済み）", "Docs Site"], "Linear プロジェクト"],
  ["ko", ["Demo Project(이미 연결됨)", "Demo Mobile(이미 연결됨)", "Docs Site"], "Linear 프로젝트"],
] as const)("a project that is bound already is marked in the brackets of %s", async (lang, texts, field) => {
  await renderWithProviders(<BindingForm onSaved={() => {}} />, { lang });
  await userEvent.click(await screen.findByRole("combobox", { name: field }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual(texts);
});

test("while the first connection to Linear is under way the project list says so, not that it failed", async () => {
  const backend = createMockBackend("connecting", { latencyMs: 0 });
  const { client } = await renderWithProviders(<BindingForm onSaved={() => {}} />, { backend });
  expect(await screen.findByText("Connecting to Linear…")).toBeInTheDocument();
  await waitFor(() => expect(client.getQueryState(queryKeys.projects)?.status).toBe("error"));
  expect(screen.queryByText("Could not read the projects from Linear")).not.toBeInTheDocument();
  // Connected: the projects are read again (the app's connection refresh does that).
  await backend.linearReconnect();
  await client.invalidateQueries({ queryKey: queryKeys.status });
  await client.invalidateQueries({ queryKey: queryKeys.projects });
  expect(await screen.findByRole("combobox", { name: "Linear project" })).toBeInTheDocument();
  expect(screen.queryByText("Connecting to Linear…")).not.toBeInTheDocument();
});

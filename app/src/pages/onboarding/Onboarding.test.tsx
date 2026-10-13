import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes } from "react-router";
import { vi } from "vitest";
import App from "@/App";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { queryKeys } from "@/hooks/queries";
import i18n, { initI18nOnce } from "@/i18n";
import ja from "@/i18n/locales/ja.json";
import ko from "@/i18n/locales/ko.json";
import zhCN from "@/i18n/locales/zh-CN.json";
import { type Backend, BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { DEMO_APP_INFO, DEMO_BINDINGS } from "@/lib/mock-fixtures";
import type { Binding, DoctorReport, LinearSetup, StatusView } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import Onboarding from "./Onboarding";

/** The guide (fresh scenario unless told otherwise); `prepare` changes the mock before the first render. */
async function start(prepare: (backend: Backend) => Promise<void> = async () => {}, scenario: "fresh" | "connecting" = "fresh") {
  const backend = createMockBackend(scenario, { latencyMs: 0 });
  await prepare(backend);
  const result = await renderWithProviders(
    <Routes>
      <Route path="/onboarding" element={<Onboarding />} />
      <Route path="/" element={<p>overview page</p>} />
    </Routes>,
    { backend, route: "/onboarding" },
  );
  return { ...result, backend };
}

test("the guide walks from language to the first binding and finishes", async () => {
  const { backend } = await start();
  const saveSettings = vi.spyOn(backend, "saveSettings");
  await userEvent.click(await screen.findByRole("radio", { name: "English" }));
  await userEvent.click(screen.getByRole("button", { name: "Next" }));

  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Authorize Linear in the browser" }));
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next" }));

  // The workspace step may name the label more than once (the check and how to add it).
  expect((await screen.findAllByText(/ai-ready/)).length).toBeGreaterThan(0);
  await userEvent.click(screen.getByRole("button", { name: "Next" }));

  expect(await screen.findByText("Claude Code")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next" }));

  await userEvent.click(await screen.findByRole("button", { name: "Bind later" }));
  expect(await screen.findByText(/assigned to you/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Start using IssueRelay" }));
  await waitFor(() => expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ onboarding_completed: true })));
  expect(await screen.findByText("overview page")).toBeInTheDocument();
});

test("the header of the guide shows the app icon beside the name", async () => {
  const { container } = await start();
  const icon = container.querySelector("header img");
  expect(icon).toHaveAttribute("src", "/icon.svg");
  expect(icon).toHaveAttribute("alt", "");
  expect(screen.getByText("IssueRelay")).toBeInTheDocument();
});

test("without a ready executor the guide does not move on", async () => {
  await start(async (backend) => {
    await backend.linearLogin();
    const report = await backend.doctor();
    vi.spyOn(backend, "doctor").mockResolvedValue({
      ...report,
      executors: report.executors.map((e) => ({ ...e, logged_in: false, problems: ["not logged in"] })),
    });
  });
  await userEvent.click(await screen.findByRole("button", { name: "Next" })); // language → Linear
  // A disabled Next ignores the click: wait until the Linear step shows the connection.
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next" })); // Linear → workspace
  await userEvent.click(await screen.findByRole("button", { name: "Next" })); // workspace → executors
  expect(await screen.findByText(/Install and log in to Claude Code or Codex first/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

// core takes one change of the login at a time: while the browser login waits, the key waits too,
// and the login can be given up.
test("a login that waits for the browser can be cancelled, and the API key waits for it", async () => {
  let cancel = () => {};
  const { backend } = await start(async (backend) => {
    backend.linearLogin = vi.fn(
      () =>
        new Promise<never>((_, reject) => {
          cancel = () => reject(new BackendError("login_cancelled"));
        }),
    );
    backend.linearCancelLogin = vi.fn(async () => cancel());
  });
  await userEvent.click(await screen.findByRole("button", { name: "Next" }));
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in the browser" }));
  expect(await screen.findByText("Finish the authorization in your browser…")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Use a personal API key" })).toBeDisabled();

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(backend.linearCancelLogin).toHaveBeenCalledTimes(1);
  expect(await screen.findByRole("button", { name: "Authorize Linear in the browser" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Use a personal API key" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("while the first connection is under way the Linear step is checking", async () => {
  await start(async () => {}, "connecting");
  await userEvent.click(await screen.findByRole("button", { name: "Next" }));
  expect(await screen.findByText("Checking the connection to Linear…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Authorize Linear in the browser" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("a check that fails is shown with a retry, and the workspace step can be skipped", async () => {
  await start(async (backend) => {
    await backend.linearLogin();
    backend.doctor = vi.fn().mockRejectedValue(new BackendError("IssueRelay could not start"));
  });
  await userEvent.click(await screen.findByRole("button", { name: "Next" }));
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(await screen.findByText("Could not run the environment check")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "I'll add them later" })).toBeEnabled();
});

// Everything below is the guide's own behaviour beyond the walk-through above.

const TITLES = [
  "Choose your language",
  "Connect Linear",
  "Check your Linear workspace",
  "Check Claude Code and Codex",
  "Bind your first project",
  "How IssueRelay works",
] as const;

/** The button that leaves a step forward, whatever the step calls it. */
const FORWARD = /^(Next|I'll add them later|Bind later)$/;

const ESC = String.fromCharCode(27);

afterEach(() => {
  // The app routes by the address's fragment. (The language is put back by the next render, in
  // `renderWithProviders` or `initI18nOnce`: doing it here would reach the components still on
  // screen, outside of `act`.)
  window.location.hash = "";
});

/** A backend whose user has logged in to Linear already (the demo's has). */
async function loggedIn(scenario: "fresh" | "demo" = "fresh"): Promise<Backend> {
  const backend = createMockBackend(scenario, { latencyMs: 0 });
  if (scenario === "fresh") await backend.linearLogin();
  return backend;
}

/** The guide on its route, with `/` as the page it ends on. `bridge` adds the app's own event bridge, which refreshes the status. */
async function guideOn(backend: Backend, { bridge = false }: { bridge?: boolean } = {}) {
  return renderWithProviders(
    <>
      {bridge && <EngineEventsBridge />}
      <Routes>
        <Route path="/onboarding" element={<Onboarding />} />
        <Route path="/" element={<p>overview page</p>} />
      </Routes>
    </>,
    { backend, route: "/onboarding" },
  );
}

/** The step with this title. */
const stepHeading = (title: string) => screen.findByRole("heading", { level: 1, name: title });

/** Presses a button once it can be pressed: a disabled button ignores a click, which would read as a step that never opened. */
async function press(name: string | RegExp) {
  await waitFor(() => expect(screen.getByRole("button", { name })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name }));
}

/** Goes forward from the language step to the step with this number (1 to 6). */
async function goTo(step: number) {
  for (let at = 1; at < step; at++) {
    await press(FORWARD);
    await stepHeading(TITLES[at]);
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

/** Opens one of a form's lists by the name of its field and picks an entry. */
async function pick(field: string, entry: string) {
  await userEvent.click(await screen.findByRole("combobox", { name: field }));
  await userEvent.click(await screen.findByRole("option", { name: entry }));
}

/** What the backend's check reports, with `patch` laid over it. */
async function reportWith(backend: Backend, patch: Partial<DoctorReport>): Promise<DoctorReport> {
  return { ...(await backend.doctor()), ...patch };
}

/** The fresh backend whose status says `linear`. */
async function backendSaying(linear: LinearSetup): Promise<Backend> {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  const view = await backend.status();
  backend.status = async (): Promise<StatusView> => ({ status: view.status, linear });
  return backend;
}

// The frame: steps, headings and buttons.

test("the guide lists its six steps with the first one current, and puts its heading in focus", async () => {
  await start();
  const steps = within(await screen.findByRole("navigation", { name: "Setup steps" }));
  const items = steps.getAllByRole("listitem");
  expect(items).toHaveLength(6);
  expect(items[0]).toHaveAttribute("aria-current", "step");
  for (const item of items.slice(1)) expect(item).not.toHaveAttribute("aria-current");
  expect(within(items[0]).getByText("Language")).toBeInTheDocument();
  expect(within(items[5]).getByText("How it works")).toBeInTheDocument();
  // Nothing has been passed yet, so there is nothing to go back to.
  expect(steps.queryAllByRole("button")).toHaveLength(0);
  expect(screen.getByText("Step 1 of 6")).toBeInTheDocument();
  expect(screen.getByText("IssueRelay")).toBeInTheDocument();
  const heading = await stepHeading(TITLES[0]);
  await waitFor(() => expect(heading).toHaveFocus());
});

test("the connection is read at the start, so that its step opens on what was read", async () => {
  const { client } = await guideOn(await loggedIn());
  await stepHeading(TITLES[0]);
  // Still on the first step, and what the status says is in the cache already.
  await waitFor(() => expect(client.getQueryData(queryKeys.status)).toBeDefined());
});

test("the heading of each step takes the focus when the step changes, forward and back", async () => {
  await start(async (backend) => {
    await backend.linearLogin();
  });
  await press("Next");
  const linear = await stepHeading(TITLES[1]);
  await waitFor(() => expect(linear).toHaveFocus());
  await press("Back");
  const language = await stepHeading(TITLES[0]);
  await waitFor(() => expect(language).toHaveFocus());
});

test("Back is not offered on the first step and goes one step back from the others", async () => {
  await start();
  expect(await screen.findByRole("button", { name: "Back" })).toBeDisabled();
  await press("Next");
  await stepHeading(TITLES[1]);
  expect(screen.getByRole("button", { name: "Back" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(await stepHeading(TITLES[0])).toBeInTheDocument();
});

test("steps already passed are marked done and can be gone back to; the steps ahead are not entries", async () => {
  await guideOn(await loggedIn());
  await goTo(3);
  const steps = within(screen.getByRole("navigation", { name: "Setup steps" }));
  expect(steps.getAllByRole("listitem")[2]).toHaveAttribute("aria-current", "step");
  expect(steps.getAllByRole("button").map((button) => button.getAttribute("aria-label"))).toEqual(["Language (done)", "Linear (done)"]);
  await userEvent.click(steps.getByRole("button", { name: "Language (done)" }));
  expect(await stepHeading(TITLES[0])).toBeInTheDocument();
  // From the first step nothing is behind, and the steps ahead are no entries.
  expect(steps.queryAllByRole("button")).toHaveLength(0);
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
});

// The language step.

test("the language step offers the system's language and the five languages by their own names", async () => {
  await start();
  await screen.findByRole("radio", { name: "English" });
  const names = ["Follow the system", "简体中文", "繁體中文", "English", "日本語", "한국어"];
  const group = screen.getByRole("radiogroup", { name: "Interface language" });
  const ids = (radios: HTMLElement[]) => radios.map((radio) => radio.id);
  expect(ids(within(group).getAllByRole("radio"))).toEqual(ids(names.map((name) => screen.getByRole("radio", { name }))));
  // A fresh install follows the system.
  expect(screen.getByRole("radio", { name: "Follow the system" })).toBeChecked();
  expect(screen.getByRole("radio", { name: "English" })).not.toBeChecked();
});

test("choosing a language switches the guide at once and saves the choice", async () => {
  const { backend } = await start();
  const saveSettings = vi.spyOn(backend, "saveSettings");
  await userEvent.click(await screen.findByRole("radio", { name: "日本語" }));
  // The guide speaks the language, and the radio shows the choice...
  expect(await screen.findByRole("button", { name: ja.onboarding.next })).toBeInTheDocument();
  expect(screen.getByRole("heading", { level: 1, name: ja.onboarding.title.language })).toBeInTheDocument();
  expect(document.documentElement.lang).toBe("ja");
  expect(screen.getByRole("radio", { name: "日本語" })).toBeChecked();
  // ...and the settings carry it, as the code of the language.
  await waitFor(() => expect(saveSettings).toHaveBeenCalledWith(expect.objectContaining({ language: "ja", onboarding_completed: false })));
  expect(saveSettings).toHaveBeenCalledTimes(1);
  await waitFor(async () => expect((await backend.settings()).language).toBe("ja"));
});

test("following the system again is saved as system and speaks the system's language", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  backend.appInfo = async () => ({ ...DEMO_APP_INFO, system_locale: "ko-KR" });
  await guideOn(backend);
  await userEvent.click(await screen.findByRole("radio", { name: "English" }));
  await waitFor(async () => expect((await backend.settings()).language).toBe("en"));
  await userEvent.click(screen.getByRole("radio", { name: "Follow the system" }));
  expect(await screen.findByRole("button", { name: ko.onboarding.next })).toBeInTheDocument();
  await waitFor(async () => expect((await backend.settings()).language).toBe("system"));
});

test("choosing the language that is chosen already saves nothing", async () => {
  const { backend } = await start();
  const saveSettings = vi.spyOn(backend, "saveSettings");
  await userEvent.click(await screen.findByRole("radio", { name: "Follow the system" }));
  expect(saveSettings).not.toHaveBeenCalled();
});

test("a choice that cannot be saved is undone and told, so that the guide never speaks a language that is not kept", async () => {
  const { backend } = await start(async (backend) => {
    backend.saveSettings = vi.fn().mockRejectedValue(new BackendError("config.toml is read-only"));
  });
  await userEvent.click(await screen.findByRole("radio", { name: "日本語" }));
  // Told in the language the guide is in again.
  expect(await screen.findByText("Could not save the settings")).toBeInTheDocument();
  expect(screen.getByText("config.toml is read-only")).toBeInTheDocument();
  expect(await stepHeading(TITLES[0])).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: "Follow the system" })).toBeChecked();
  expect(screen.getByRole("radio", { name: "日本語" })).not.toBeChecked();
  expect(document.documentElement.lang).toBe("en");
  expect((await backend.settings()).language).toBe("system");
});

test("settings that cannot be read are an error with a retry on the language step, and the guide goes on", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  const read = backend.settings.bind(backend);
  backend.settings = vi.fn().mockRejectedValueOnce(new BackendError("the config file is unreadable")).mockImplementation(read);
  await guideOn(backend);
  expect(await screen.findByText("Could not read the settings")).toBeInTheDocument();
  expect(screen.getByText("the config file is unreadable")).toBeInTheDocument();
  expect(screen.queryByRole("radio")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("radio", { name: "English" })).toBeInTheDocument();
});

test("the whole panel follows a language chosen in the guide, and does not fall back to the old one on the way", async () => {
  // The app follows the language its settings say (LanguageSync): the guide has to tell the
  // settings first, or the app sets the old language back until the save has been read again.
  await initI18nOnce("en");
  const changes: string[] = [];
  const record = (language: string) => changes.push(language);
  i18n.on("languageChanged", record);
  window.location.hash = "#/onboarding";
  try {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    const read = vi.spyOn(backend, "settings");
    render(<App backend={backend} />);
    await userEvent.click(await screen.findByRole("radio", { name: "日本語" }));
    expect(await screen.findByRole("button", { name: ja.onboarding.next })).toBeInTheDocument();
    // The settings are read again once they are saved; the panel keeps the language they now say.
    await waitFor(() => expect(read.mock.calls.length).toBeGreaterThanOrEqual(2));
    expect(changes).toEqual(["ja"]);
    expect(document.documentElement.lang).toBe("ja");
  } finally {
    i18n.off("languageChanged", record);
  }
});

// The Linear step.

test("a connection that exists shows who authorized it and lets the guide go on", async () => {
  await guideOn(await loggedIn());
  await press("Next");
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  expect(screen.getByText("Connected as")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  // Nothing to put a key into.
  expect(screen.queryByRole("button", { name: "Use a personal API key" })).not.toBeInTheDocument();
});

test("without a login the step offers the browser login and, apart from it, the key; the guide waits", async () => {
  await guideOn(createMockBackend("fresh", { latencyMs: 0 }));
  await press("Next");
  expect(await screen.findByText("Not logged in to Linear")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Authorize Linear in the browser" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Use a personal API key" })).toBeEnabled();
  expect(screen.queryByLabelText("Personal API key")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("the personal API key opens on request, and a key that Linear takes connects the guide", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  const useKey = vi.spyOn(backend, "linearUseApiKey");
  await guideOn(backend);
  await press("Next");
  const toggle = await screen.findByRole("button", { name: "Use a personal API key" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  await userEvent.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  const field = await screen.findByLabelText("Personal API key");
  expect(field).toHaveAttribute("type", "password");
  await userEvent.type(field, "lin_api_demo");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  expect(useKey).toHaveBeenCalledWith("lin_api_demo");
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
});

test("a key that Linear refuses is told, and the guide stays unconnected", async () => {
  await guideOn(createMockBackend("fresh", { latencyMs: 0 }));
  await press("Next");
  await userEvent.click(await screen.findByRole("button", { name: "Use a personal API key" }));
  await userEvent.type(await screen.findByLabelText("Personal API key"), "lin_api_bad");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  expect(await screen.findByText("Could not use the API key")).toBeInTheDocument();
  expect(screen.getByText("Linear did not accept the API key")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("a connection that failed says why, offers to try again or to authorize again, and stops the guide", async () => {
  await guideOn(await backendSaying({ kind: "connect_failed", message: "401 Unauthorized" }));
  await press("Next");
  expect(await screen.findByText("Could not connect to Linear")).toBeInTheDocument();
  expect(screen.getByText("401 Unauthorized")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Try again" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Authorize again" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Use a personal API key" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("tools of Linear that changed are named, with nothing to press and no way on", async () => {
  await guideOn(await backendSaying({ kind: "interface_changed", message: "tool list_issues has no cursor" }));
  await press("Next");
  expect(await screen.findByText("Linear's tools changed: update IssueRelay")).toBeInTheDocument();
  expect(screen.getByText("tool list_issues has no cursor")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Use a personal API key" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("a status that cannot be read is an error with a retry on the Linear step", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  const read = backend.status.bind(backend);
  backend.status = vi.fn().mockRejectedValue(new BackendError("the host is not answering"));
  await guideOn(backend);
  await press("Next");
  expect(await screen.findByText("Could not read IssueRelay's status")).toBeInTheDocument();
  expect(screen.getByText("the host is not answering")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  backend.status = read;
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("button", { name: "Authorize Linear in the browser" })).toBeInTheDocument();
});

test("when the first connection ends the step shows the result, without leaving it", async () => {
  const backend = createMockBackend("connecting", { latencyMs: 0 });
  await guideOn(backend, { bridge: true });
  await press("Next");
  expect(await screen.findByText("Checking the connection to Linear…")).toBeInTheDocument();
  // The host's connection comes through: the mock connects and tells the panel with a Status event.
  await backend.linearReconnect();
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  expect(screen.queryByText("Checking the connection to Linear…")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
});

test("while the first connection is under way neither the browser login nor the key is offered", async () => {
  await guideOn(createMockBackend("connecting", { latencyMs: 0 }));
  await press("Next");
  await screen.findByText("Checking the connection to Linear…");
  expect(screen.queryByRole("button", { name: "Use a personal API key" })).not.toBeInTheDocument();
  expect(within(screen.getByRole("main")).getAllByRole("button").map((button) => button.textContent)).toEqual(["Back", "Next"]);
});

// The workspace step.

test("a workspace that has what IssueRelay needs is confirmed, and the guide goes on with Next", async () => {
  await guideOn(await loggedIn());
  await goTo(3);
  expect(await screen.findByText("The label ai-ready exists")).toBeInTheDocument();
  expect(screen.getByText("The statuses IssueRelay uses exist")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
});

test("a missing label or status is explained and does not stop the guide, which then says it will be added later", async () => {
  const backend = await loggedIn();
  vi.spyOn(backend, "doctor").mockResolvedValue(
    await reportWith(backend, { workspace: { label_ok: false, missing_states: ["In Testing"], teams_checked: ["Demo"] } }),
  );
  await guideOn(backend);
  await goTo(3);
  expect(await screen.findByText("The label ai-ready is missing")).toBeInTheDocument();
  expect(screen.getByText("Missing statuses: In Testing")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Open Linear" })).toBeEnabled();
  expect(screen.queryByRole("button", { name: "Next" })).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "I'll add them later" }));
  expect(await stepHeading(TITLES[3])).toBeInTheDocument();
});

test("the label the workspace is checked for is the one the settings name", async () => {
  const backend = await loggedIn();
  const settings = await backend.settings();
  await backend.saveSettings({ ...settings, trigger: { ...settings.trigger, label: "robot-ready" } });
  vi.spyOn(backend, "doctor").mockResolvedValue(
    await reportWith(backend, { workspace: { label_ok: false, missing_states: [], teams_checked: ["Demo"] } }),
  );
  await guideOn(backend);
  await goTo(3);
  expect(await screen.findByText("The label robot-ready is missing")).toBeInTheDocument();
  expect(screen.getByText(/needs the robot-ready label/)).toBeInTheDocument();
});

test("while the check runs the step says it takes up to a minute and does not hold the guide", async () => {
  const backend = await loggedIn();
  backend.doctor = vi.fn(() => new Promise<DoctorReport>(() => {}));
  await guideOn(backend);
  await goTo(3);
  expect(await screen.findByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.getByText("Checking Linear, Claude Code and Codex. This takes up to a minute.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
});

test("without a connection the workspace cannot be checked, and the step says why", async () => {
  const backend = await loggedIn();
  vi.spyOn(backend, "doctor").mockResolvedValue(
    await reportWith(backend, { workspace: null, linear_error: "not connected to Linear: log in first" }),
  );
  await guideOn(backend);
  await goTo(3);
  expect(await screen.findByText("Connect to Linear first to check the workspace")).toBeInTheDocument();
  expect(screen.getByText("The check of Linear failed")).toBeInTheDocument();
  expect(screen.getByText("not connected to Linear: log in first")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "I'll add them later" })).toBeEnabled();
});

test("a failed check is tried again with the retry, and the report replaces the error", async () => {
  const backend = await loggedIn();
  const read = backend.doctor.bind(backend);
  backend.doctor = vi.fn().mockRejectedValueOnce(new BackendError("IssueRelay could not start")).mockImplementation(read);
  await guideOn(backend);
  await goTo(3);
  expect(await screen.findByText("Could not run the environment check")).toBeInTheDocument();
  expect(screen.getByText("IssueRelay could not start")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByText("The label ai-ready exists")).toBeInTheDocument();
  expect(screen.queryByText("Could not run the environment check")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
});

test("Check again asks again and keeps the report in view meanwhile", async () => {
  const backend = await loggedIn();
  const read = backend.doctor.bind(backend);
  const again = deferred<DoctorReport>();
  backend.doctor = vi.fn().mockImplementationOnce(read).mockImplementationOnce(() => again.promise);
  await guideOn(backend);
  await goTo(3);
  await userEvent.click(await screen.findByRole("button", { name: "Check again" }));
  expect(await screen.findByText("Checking Linear, Claude Code and Codex. This takes up to a minute.")).toBeInTheDocument();
  expect(screen.getByText("The label ai-ready exists")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Check again" })).toBeDisabled();
  const report = await read();
  await act(async () => again.resolve(report));
  await waitFor(() => expect(screen.queryByText("Checking Linear, Claude Code and Codex. This takes up to a minute.")).not.toBeInTheDocument());
  expect(screen.getByRole("button", { name: "Check again" })).toBeEnabled();
});

test("the environment is checked once for the workspace and the executors, and again when they are entered afresh", async () => {
  const backend = await loggedIn();
  const doctor = vi.spyOn(backend, "doctor");
  await guideOn(backend);
  await goTo(4);
  expect(doctor).toHaveBeenCalledTimes(1);
  await userEvent.click(screen.getByRole("button", { name: "Back" }));
  await stepHeading(TITLES[2]);
  await press("Next");
  await stepHeading(TITLES[3]);
  expect(doctor).toHaveBeenCalledTimes(1);
  // Out of the two steps and back in: the user may have mended something meanwhile.
  await press("Next");
  await stepHeading(TITLES[4]);
  await userEvent.click(screen.getByRole("button", { name: "Back" }));
  await stepHeading(TITLES[3]);
  await waitFor(() => expect(doctor).toHaveBeenCalledTimes(2));
});

// The executors step.

test("both executors are shown with what is missing in each, and one that is ready is enough", async () => {
  await guideOn(await loggedIn());
  await goTo(4);
  const claude = await screen.findByRole("region", { name: "Claude Code" });
  expect(within(claude).getByText("Logged in")).toBeInTheDocument();
  const codex = screen.getByRole("region", { name: "Codex" });
  expect(within(codex).getByText("Not logged in")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  expect(screen.queryByText(/Install and log in to Claude Code or Codex first/)).not.toBeInTheDocument();
});

test("with no executor ready the step says what to do, and the guide stays on it", async () => {
  const backend = await loggedIn();
  const report = await backend.doctor();
  vi.spyOn(backend, "doctor").mockResolvedValue({
    ...report,
    executors: report.executors.map((e) => ({ ...e, program: null, version: null, logged_in: false, linear_mcp: false })),
  });
  await guideOn(backend);
  await goTo(4);
  expect(await screen.findByText("Install and log in to Claude Code or Codex first")).toBeInTheDocument();
  expect(screen.getByText("Follow what each card says below, then check again.")).toBeInTheDocument();
  expect(screen.getAllByText("Not installed")).toHaveLength(2);
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("an executor that is installed and logged in but not connected to Linear is not ready", async () => {
  const backend = await loggedIn();
  const report = await backend.doctor();
  vi.spyOn(backend, "doctor").mockResolvedValue({
    ...report,
    executors: report.executors.map((e) => ({ ...e, logged_in: true, linear_mcp: false })),
  });
  await guideOn(backend);
  await goTo(4);
  expect(await screen.findByText("Install and log in to Claude Code or Codex first")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
});

test("a check that failed keeps the guide on the executors step, with a retry", async () => {
  const backend = await loggedIn();
  backend.doctor = vi.fn().mockRejectedValue(new BackendError("IssueRelay could not start"));
  await guideOn(backend);
  await goTo(4);
  expect(await screen.findByText("Could not run the environment check")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  expect(screen.queryByText(/Install and log in to Claude Code or Codex first/)).not.toBeInTheDocument();
  // Mended: the next try finds the report.
  const mended = await createMockBackend("demo", { latencyMs: 0 }).doctor();
  backend.doctor = vi.fn().mockResolvedValue(mended);
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
  expect(screen.queryByText("Could not run the environment check")).not.toBeInTheDocument();
});

test("authorizing Linear in an executor from the guide checks again, and the executor then counts", async () => {
  const backend = await loggedIn();
  const read = backend.doctor.bind(backend);
  let authorized = false;
  backend.doctor = vi.fn(async () => {
    const report = await read();
    return { ...report, executors: report.executors.map((e) => (e.kind === "claude" ? { ...e, linear_mcp: authorized } : e)) };
  });
  backend.setupAction = vi.fn(async () => {
    authorized = true;
  });
  await guideOn(backend);
  await goTo(4);
  expect(await screen.findByText("Install and log in to Claude Code or Codex first")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in Claude Code" }));
  expect(backend.setupAction).toHaveBeenCalledWith({ kind: "login_linear_mcp", executor: "claude" });
  await waitFor(() => expect(screen.getByRole("button", { name: "Next" })).toBeEnabled());
  expect(screen.queryByText("Install and log in to Claude Code or Codex first")).not.toBeInTheDocument();
  expect(backend.doctor).toHaveBeenCalledTimes(2);
});

// The binding step.

test("the first binding is made in the guide: the risk of full access is asked once, then the binding is listed and another can follow", async () => {
  const backend = await loggedIn();
  const saveBinding = vi.spyOn(backend, "saveBinding");
  const saveSettings = vi.spyOn(backend, "saveSettings");
  await guideOn(backend);
  await goTo(5);
  expect(await screen.findByRole("button", { name: "Bind later" })).toBeEnabled();
  await pick("Linear project", "Demo Project");
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const warning = await screen.findByRole("alertdialog");
  expect(saveBinding).not.toHaveBeenCalled();
  await userEvent.click(within(warning).getByRole("button", { name: "I understand" }));

  const saved = await screen.findByText("The binding is saved");
  expect(saveBinding).toHaveBeenCalledWith(expect.objectContaining({ project_id: "proj-1", directory: "~/code/new-project" }));
  expect(saveSettings).toHaveBeenCalledWith(expect.objectContaining({ full_access_acknowledged: true }));
  const list = screen.getByRole("list", { name: "Bound projects" });
  expect(within(list).getByText("Demo Project")).toBeInTheDocument();
  expect(within(list).getByText("~/code/new-project")).toBeInTheDocument();
  // The form is gone and the focus did not fall to the page: it is on the news.
  expect(screen.queryByRole("combobox", { name: "Linear project" })).not.toBeInTheDocument();
  await waitFor(() => expect(saved).toHaveFocus());
  expect(screen.queryByRole("button", { name: "Bind later" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();

  await userEvent.click(screen.getByRole("button", { name: "Add another" }));
  const again = await screen.findByRole("heading", { level: 2, name: "Add another binding" });
  await waitFor(() => expect(again).toHaveFocus());
  await userEvent.click(await screen.findByRole("combobox", { name: "Linear project" }));
  // A project that is bound cannot be bound twice.
  expect(await screen.findByRole("option", { name: "Demo Project (already bound)" })).toHaveAttribute("aria-disabled", "true");
  await userEvent.keyboard("{Escape}");
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  const add = await screen.findByRole("button", { name: "Add another" });
  await waitFor(() => expect(add).toHaveFocus());
  await userEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(await stepHeading(TITLES[5])).toBeInTheDocument();
  // The risk was told once: the settings carry the confirmation once.
  expect(saveSettings.mock.calls.filter(([settings]) => settings.full_access_acknowledged)).toHaveLength(1);
});

test("the warning of full access, closed without an answer, gives the focus back to Save and binds nothing", async () => {
  const backend = await loggedIn();
  const saveBinding = vi.spyOn(backend, "saveBinding");
  await guideOn(backend);
  await goTo(5);
  await pick("Linear project", "Demo Project");
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const warning = await screen.findByRole("alertdialog");
  await userEvent.click(within(warning).getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  // The dialog is opened by the form, not by a trigger of its own: the focus is put back where it was.
  await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toHaveFocus());
  expect(saveBinding).not.toHaveBeenCalled();
  expect(screen.queryByText("The binding is saved")).not.toBeInTheDocument();
});

test("a second binding is made in the guide too, and listed with the first", async () => {
  const backend = await loggedIn();
  const settings = await backend.settings();
  await backend.saveSettings({ ...settings, full_access_acknowledged: true });
  await guideOn(backend);
  await goTo(5);
  await pick("Linear project", "Demo Project");
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByText("The binding is saved");
  await userEvent.click(screen.getByRole("button", { name: "Add another" }));
  await pick("Linear project", "Docs Site");
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(within(screen.getByRole("list", { name: "Bound projects" })).getAllByRole("listitem")).toHaveLength(2));
  expect(screen.getByText("The binding is saved")).toBeInTheDocument();
});

test("bindings that exist already are listed instead of a form, and the guide says Next", async () => {
  await guideOn(await loggedIn("demo"));
  await goTo(5);
  const list = await screen.findByRole("list", { name: "Bound projects" });
  expect(within(list).getAllByRole("listitem")).toHaveLength(DEMO_BINDINGS.length);
  expect(within(list).getByText("Demo Mobile")).toBeInTheDocument();
  expect(within(list).getByText("~/code/demo-mobile")).toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Linear project" })).not.toBeInTheDocument();
  expect(screen.queryByText("The binding is saved")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  expect(screen.queryByRole("button", { name: "Bind later" })).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Add another" }));
  expect(await screen.findByRole("combobox", { name: "Linear project" })).toBeInTheDocument();
});

test("names and folders of bound projects are shown as cleaned text", async () => {
  const backend = await loggedIn("demo");
  const bindings: Binding[] = [{ ...DEMO_BINDINGS[0], project_name: `Demo${ESC}[31m Project`, directory: "~/code/demo-app" }];
  backend.bindings = async () => bindings;
  await guideOn(backend);
  await goTo(5);
  const list = await screen.findByRole("list", { name: "Bound projects" });
  expect(within(list).getByText("Demo Project")).toBeInTheDocument();
  expect(list.textContent).not.toContain("[31m");
});

test("bindings that cannot be read are an error with a retry on the binding step", async () => {
  const backend = await loggedIn();
  const read = backend.bindings.bind(backend);
  backend.bindings = vi.fn().mockRejectedValueOnce(new BackendError("the config file is unreadable")).mockImplementation(read);
  await guideOn(backend);
  await goTo(5);
  expect(await screen.findByText("Could not read the bindings")).toBeInTheDocument();
  expect(screen.getByText("the config file is unreadable")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Bind later" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByRole("combobox", { name: "Linear project" })).toBeInTheDocument();
});

test("a binding the engine refuses is told in the form, and the guide stays on the form", async () => {
  const backend = await loggedIn();
  backend.chooseDirectory = vi.fn().mockResolvedValue("code/new-project");
  const settings = await backend.settings();
  await backend.saveSettings({ ...settings, full_access_acknowledged: true });
  await guideOn(backend);
  await goTo(5);
  await pick("Linear project", "Demo Project");
  await userEvent.click(screen.getByRole("button", { name: "Choose…" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const alert = await screen.findByRole("alert");
  expect(within(alert).getByText("Could not save the binding")).toBeInTheDocument();
  expect(screen.queryByText("The binding is saved")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Bind later" })).toBeEnabled();
});

// The last step.

test("the last step tells the four conditions, one example, how an issue moves and how the control server is used", async () => {
  await guideOn(await loggedIn());
  await goTo(6);
  const conditions = screen.getByRole("region", { name: "Trigger conditions" });
  expect(within(conditions).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    "The issue is assigned to you",
    "It has the ai-ready label",
    "Its status is Todo",
    "Its project is bound to a local folder and the executor is ready",
  ]);
  const example = screen.getByRole("group", { name: "A request the AI can finish" });
  expect(within(example).getByText("Add a /health endpoint that returns the app version")).toBeInTheDocument();
  expect(within(example).getByText(/GET \/health answers with status 200/)).toBeInTheDocument();
  // One example is enough here; the help page has the other.
  expect(screen.queryByText("Make the app faster")).not.toBeInTheDocument();
  const flow = screen.getByRole("region", { name: "How an issue moves" });
  expect(within(flow).getByRole("list", { name: "The statuses of an issue in Linear" })).toBeInTheDocument();
  expect(within(flow).getByRole("list", { name: "What the statuses in the panel mean" }).children).toHaveLength(8);
  expect(
    screen.getByText(
      "In Settings you can let Claude Code and Codex use IssueRelay's MCP control server. It is registered for your user, so unattended runs load it too, and its tools can pause monitoring.",
    ),
  ).toBeInTheDocument();
});

test("the trigger names in the last step are the ones of the settings", async () => {
  const backend = await loggedIn();
  const settings = await backend.settings();
  await backend.saveSettings({ ...settings, trigger: { ...settings.trigger, label: "robot-ready", todo: "Ready" } });
  await guideOn(backend);
  await goTo(6);
  expect(screen.getByText("It has the robot-ready label")).toBeInTheDocument();
  expect(screen.getByText("Its status is Ready")).toBeInTheDocument();
});

test("a guide that ends with no binding says that nothing will be taken on until one is made", async () => {
  await guideOn(await loggedIn());
  await goTo(6);
  expect(await screen.findByText(/No project is bound yet/)).toBeInTheDocument();
});

test("a guide that ends with a binding does not say so", async () => {
  await guideOn(await loggedIn("demo"));
  await goTo(6);
  await screen.findByText("It has the ai-ready label");
  expect(screen.queryByText(/No project is bound yet/)).not.toBeInTheDocument();
});

test("starting saves the settings with the guide completed, keeping everything else, and goes to the overview", async () => {
  const backend = await loggedIn();
  const saveSettings = vi.spyOn(backend, "saveSettings");
  const before = await backend.settings();
  await guideOn(backend);
  await goTo(6);
  await userEvent.click(screen.getByRole("button", { name: "Start using IssueRelay" }));
  expect(await screen.findByText("overview page")).toBeInTheDocument();
  expect(saveSettings).toHaveBeenCalledTimes(1);
  expect(saveSettings).toHaveBeenCalledWith({ ...before, onboarding_completed: true });
});

test("a save that fails keeps the guide on its last step and says why; trying again goes on", async () => {
  const backend = await loggedIn();
  const save = backend.saveSettings.bind(backend);
  backend.saveSettings = vi
    .fn()
    .mockRejectedValueOnce(new BackendError("config.toml was changed while IssueRelay was running"))
    .mockImplementation(save);
  await guideOn(backend);
  await goTo(6);
  await userEvent.click(screen.getByRole("button", { name: "Start using IssueRelay" }));
  expect(await screen.findByText("Could not save the settings")).toBeInTheDocument();
  expect(screen.getByText("config.toml was changed while IssueRelay was running")).toBeInTheDocument();
  expect(screen.queryByText("overview page")).not.toBeInTheDocument();
  expect(screen.getByRole("heading", { level: 1, name: TITLES[5] })).toBeInTheDocument();
  await waitFor(() => expect(screen.getByRole("button", { name: "Start using IssueRelay" })).toBeEnabled());
  expect((await backend.settings()).onboarding_completed).toBe(false);
  await userEvent.click(screen.getByRole("button", { name: "Start using IssueRelay" }));
  expect(await screen.findByText("overview page")).toBeInTheDocument();
});

test("while the settings are being saved nothing can take the user off the last step", async () => {
  const backend = await loggedIn();
  const save = deferred<void>();
  backend.saveSettings = vi.fn(() => save.promise);
  await guideOn(backend);
  await goTo(6);
  await userEvent.click(screen.getByRole("button", { name: "Start using IssueRelay" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Start using IssueRelay" })).toBeDisabled());
  expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();
  const steps = within(screen.getByRole("navigation", { name: "Setup steps" }));
  expect(steps.getAllByRole("button")).toHaveLength(5);
  for (const button of steps.getAllByRole("button")) expect(button).toBeDisabled();
  await act(async () => save.resolve());
  expect(await screen.findByText("overview page")).toBeInTheDocument();
});

test("settings that cannot be read keep the guide from finishing, with a retry", async () => {
  const backend = await loggedIn();
  const read = backend.settings.bind(backend);
  backend.settings = vi.fn().mockRejectedValue(new BackendError("the config file is unreadable"));
  await guideOn(backend);
  await goTo(6);
  expect(await screen.findByText("Could not read the settings")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Start using IssueRelay" })).toBeDisabled();
  backend.settings = read;
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Start using IssueRelay" })).toBeEnabled());
  expect(screen.queryByText("Could not read the settings")).not.toBeInTheDocument();
});

// The guide in the app.

/** The app on this address, with this backend. */
function openApp(hash: string, backend: Backend) {
  window.location.hash = hash;
  return render(<App backend={backend} />);
}

/** Waits until the overview has read what it shows, so that nothing arrives after the test (outside `act`). */
async function overviewSettled() {
  expect(await screen.findByRole("heading", { level: 1, name: "Overview" })).toBeInTheDocument();
  await screen.findByText("Version 0.1.0");
  await waitFor(() => expect(screen.queryByRole("status", { name: "Loading…" })).not.toBeInTheDocument());
}

test("a first run is led through the guide into the app, and the first-run gate lets it stay", async () => {
  await initI18nOnce("en");
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  const saveSettings = vi.spyOn(backend, "saveSettings");
  openApp("", backend);
  await stepHeading(TITLES[0]);
  expect(window.location.hash).toBe("#/onboarding");
  await press("Next");
  await userEvent.click(await screen.findByRole("button", { name: "Authorize Linear in the browser" }));
  await screen.findByText("Jane Doe");
  await press("Next");
  await screen.findByText("The label ai-ready exists");
  await press("Next");
  await screen.findByText("Claude Code");
  await press("Next");
  await press("Bind later");
  await userEvent.click(await screen.findByRole("button", { name: "Start using IssueRelay" }));
  await overviewSettled();
  expect(window.location.hash).toBe("#/");
  expect(screen.getByRole("link", { name: "Runs" })).toBeInTheDocument();
  // The gate read the saved settings, not the ones from before: the guide does not come back.
  expect(screen.queryByRole("heading", { level: 1, name: TITLES[0] })).not.toBeInTheDocument();
  expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({ onboarding_completed: true }));
});

test("the environment check leads back to the guide, which starts at the first step and shows how things stand", async () => {
  await initI18nOnce("en");
  openApp("#/doctor", createMockBackend("demo", { latencyMs: 0 }));
  await userEvent.click(await screen.findByRole("link", { name: "Run the first-run guide again" }));
  expect(await stepHeading(TITLES[0])).toBeInTheDocument();
  expect(window.location.hash).toBe("#/onboarding");
  // No sidebar around it.
  expect(screen.queryByRole("link", { name: "Runs" })).not.toBeInTheDocument();
  expect(screen.getByRole("radio", { name: "Follow the system" })).toBeChecked();
  await press("Next");
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  await press("Next");
  expect(await screen.findByText("The label ai-ready exists")).toBeInTheDocument();
  await press("Next");
  // The demo's Codex is not logged in, and the guide says so.
  expect(await screen.findByText("Claude Code")).toBeInTheDocument();
  expect(within(screen.getByRole("region", { name: "Codex" })).getByText("Not logged in")).toBeInTheDocument();
  await press("Next");
  expect(await screen.findByText("Demo Mobile")).toBeInTheDocument();
  await press("Next");
  await userEvent.click(await screen.findByRole("button", { name: "Start using IssueRelay" }));
  await overviewSettled();
});

// The guide in the other languages.

test.each([
  ["zh-CN", zhCN],
  ["ja", ja],
  ["ko", ko],
] as const)("the first steps read in %s", async (lang, table) => {
  const backend = await loggedIn();
  await renderWithProviders(
    <Routes>
      <Route path="/onboarding" element={<Onboarding />} />
    </Routes>,
    { backend, route: "/onboarding", lang },
  );
  expect(await screen.findByRole("heading", { level: 1, name: table.onboarding.title.language })).toBeInTheDocument();
  expect(screen.getByRole("navigation", { name: table.onboarding.steps_label })).toBeInTheDocument();
  // The names of the languages are the same whatever the interface speaks.
  expect(await screen.findByRole("radio", { name: "English" })).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: table.settings.follow_system })).toBeChecked();
  await userEvent.click(screen.getByRole("button", { name: table.onboarding.next }));
  expect(await screen.findByRole("heading", { level: 1, name: table.onboarding.title.linear })).toBeInTheDocument();
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  expect(screen.getByText(table.onboarding.step_of.replace("{{current}}", "2").replace("{{total}}", "6"))).toBeInTheDocument();
});

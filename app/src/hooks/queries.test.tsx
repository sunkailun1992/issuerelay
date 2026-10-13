import { useQueryClient } from "@tanstack/react-query";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { toast } from "sonner";
import { vi } from "vitest";
import { BackendError, type Backend } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { Binding, Settings } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { advanceUntil } from "@/test/time";
import {
  queryKeys,
  useAppInfo,
  useAutostart,
  useBindings,
  useCancelJob,
  useControlServers,
  useDoctor,
  useIssues,
  useJob,
  useJobLog,
  useJobs,
  useLinearApiKey,
  useLinearLogin,
  useLinearLoginWaiting,
  useLinearLogout,
  useLinearReconnect,
  useLinearServerNamed,
  useMarkDone,
  usePause,
  usePollNow,
  useProjects,
  useRemoveBinding,
  useResume,
  useRetryJob,
  useSaveBinding,
  useSaveSettings,
  useSetAutostart,
  useSetupAction,
  useSettings,
  useStatus,
} from "./queries";

afterEach(() => {
  vi.useRealTimers();
});

/** The keys a client was told to invalidate, as strings. */
function invalidated(spy: { mock: { calls: unknown[][] } }): string[] {
  return spy.mock.calls.map(([filters]) => JSON.stringify((filters as { queryKey?: unknown } | undefined)?.queryKey));
}

describe("the query keys", () => {
  test("are the ones the engine events invalidate", () => {
    expect(queryKeys.status).toEqual(["status"]);
    expect(queryKeys.jobs).toEqual(["jobs"]);
    expect(queryKeys.job(7)).toEqual(["job", 7]);
    expect(queryKeys.log(7)).toEqual(["log", 7]);
    expect(queryKeys.issues).toEqual(["issues"]);
    expect(queryKeys.settings).toEqual(["settings"]);
    expect(queryKeys.bindings).toEqual(["bindings"]);
    expect(queryKeys.linearServerNamed("codex")).toEqual(["linearServerNamed", "codex"]);
  });

  test("are different for everything that is read", () => {
    const keys = [
      queryKeys.startup, queryKeys.status, queryKeys.jobs, queryKeys.job(1), queryKeys.log(1), queryKeys.issues,
      queryKeys.bindings, queryKeys.projects, queryKeys.settings, queryKeys.doctor, queryKeys.controlServers,
      queryKeys.linearServerNamed("claude"), queryKeys.linearServerNamed("codex"), queryKeys.autostart, queryKeys.appInfo,
    ].map((key) => JSON.stringify(key));
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("reading", () => {
  function Reader() {
    const status = useStatus();
    const jobs = useJobs();
    const job = useJob(12);
    const issues = useIssues();
    const info = useAppInfo();
    const bindings = useBindings();
    const settings = useSettings();
    const autostart = useAutostart();
    const servers = useControlServers();
    return (
      <p>
        {[
          status.data?.linear.kind,
          jobs.data?.length,
          job.data?.issue_identifier,
          issues.data?.length,
          info.data?.version,
          bindings.data?.length,
          settings.data?.poll_interval_secs,
          autostart.data === undefined ? undefined : String(autostart.data),
          servers.data?.claude,
        ].join(" ")}
      </p>
    );
  }

  test("the hooks read from the backend they are given", async () => {
    await renderWithProviders(<Reader />);
    expect(await screen.findByText("connected 9 DEMO-12 9 0.1.0 2 30 false not_registered")).toBeInTheDocument();
  });

  test("jobs are asked for with a limit of two hundred unless another is given", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const list = vi.spyOn(backend, "listJobs");
    function Both() {
      useJobs();
      useJobs(5);
      return null;
    }
    await renderWithProviders(<Both />, { backend });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    expect(list.mock.calls.map(([limit]) => limit).sort((a, b) => a - b)).toEqual([5, 200]);
  });

  test("projects, the environment check and a server's name are read only when asked for", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const projects = vi.spyOn(backend, "projects");
    const doctor = vi.spyOn(backend, "doctor");
    const named = vi.spyOn(backend, "linearServerNamed");
    function Off() {
      // A read that is not switched off: when it has been answered, so would the others have been.
      const status = useStatus();
      useProjects(false);
      useDoctor(false);
      useLinearServerNamed("claude", false);
      return <p>{status.data ? "status read" : "reading"}</p>;
    }
    function On() {
      useProjects(true);
      useDoctor(true);
      useLinearServerNamed("codex", true);
      return null;
    }
    const off = await renderWithProviders(<Off />, { backend });
    expect(await screen.findByText("status read")).toBeInTheDocument();
    expect(projects).not.toHaveBeenCalled();
    expect(doctor).not.toHaveBeenCalled();
    expect(named).not.toHaveBeenCalled();
    off.unmount();
    await renderWithProviders(<On />, { backend });
    await waitFor(() => expect(named).toHaveBeenCalledWith("codex"));
    expect(projects).toHaveBeenCalledTimes(1);
    expect(doctor).toHaveBeenCalledTimes(1);
  });

  test("a job's log is read again every second and a half while it is live, and not otherwise", async () => {
    // Every timer is the test's: the mock answers from a timer and so does the library's redraw, so
    // time is advanced by hand and nothing is waited for.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const log = vi.spyOn(backend, "jobLog");
    function Logs() {
      const [live, setLive] = useState(true);
      useJobLog(12, { live });
      return (
        <button type="button" onClick={() => setLive(false)}>
          finish
        </button>
      );
    }
    const reads = () => log.mock.calls.length;

    await renderWithProviders(<Logs />, { backend });
    expect(reads()).toBe(1);
    expect(log.mock.calls[0][0]).toBe(12);
    // The next read comes a second and a half later, not sooner, and then again.
    expect(await advanceUntil(() => reads() === 2)).toBe(1500);
    expect(await advanceUntil(() => reads() === 3)).toBeGreaterThanOrEqual(1500);
    expect(await advanceUntil(() => reads() === 4)).toBeLessThan(1700);

    fireEvent.click(screen.getByRole("button", { name: "finish" }));
    await act(() => vi.advanceTimersByTimeAsync(0));
    const before = reads();
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(reads()).toBe(before);
  });
});

describe("acting", () => {
  function Act({ label = "go", run }: { label?: string; run: () => void }) {
    return (
      <button type="button" onClick={run}>
        {label}
      </button>
    );
  }

  /** Clicks `label` and returns the keys that were invalidated by the time it settled. */
  async function keysAfterClicking(label: string, spy: { mock: { calls: unknown[][] }; mockClear(): void }) {
    spy.mockClear();
    await userEvent.click(screen.getByRole("button", { name: label }));
    await waitFor(() => expect(spy.mock.calls.length).toBeGreaterThan(0));
    return invalidated(spy);
  }

  const key = (...parts: unknown[]) => JSON.stringify(parts);

  test("each action refreshes the data it changes", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    function Everything() {
      const pause = usePause();
      const resume = useResume();
      const pollNow = usePollNow();
      const cancel = useCancelJob();
      const retry = useRetryJob();
      const markDone = useMarkDone();
      const saveBinding = useSaveBinding();
      const removeBinding = useRemoveBinding();
      const saveSettings = useSaveSettings();
      const apiKey = useLinearApiKey();
      const logout = useLinearLogout();
      const reconnect = useLinearReconnect();
      const setup = useSetupAction();
      const autostart = useSetAutostart();
      return (
        <>
          <Act label="pause" run={() => pause.mutate()} />
          <Act label="resume" run={() => resume.mutate()} />
          <Act label="poll now" run={() => pollNow.mutate()} />
          <Act label="cancel" run={() => cancel.mutate(12)} />
          <Act label="retry" run={() => retry.mutate(8)} />
          <Act label="mark done" run={() => markDone.mutate(10)} />
          <Act
            label="save binding"
            run={() => saveBinding.mutate({ id: "proj-3", project_id: "proj-3", project_name: "Docs Site", directory: "~/code/docs" })}
          />
          <Act label="remove binding" run={() => removeBinding.mutate("proj-1")} />
          <Act label="save settings" run={() => void backend.settings().then((settings) => saveSettings.mutate(settings))} />
          <Act label="api key" run={() => apiKey.mutate("lin_api_demo")} />
          <Act label="log out" run={() => logout.mutate()} />
          <Act label="reconnect" run={() => reconnect.mutate()} />
          <Act label="setup" run={() => setup.mutate({ kind: "add_linear_mcp", executor: "claude" })} />
          <Act label="autostart" run={() => autostart.mutate(true)} />
        </>
      );
    }
    const { client } = await renderWithProviders(<Everything />, { backend });
    const spy = vi.spyOn(client, "invalidateQueries");
    const expectations: [string, string[]][] = [
      ["pause", [key("status"), key("settings")]],
      ["resume", [key("status"), key("settings")]],
      ["poll now", [key("status"), key("jobs"), key("issues")]],
      ["cancel", [key("status"), key("jobs"), key("job", 12), key("log", 12), key("issues")]],
      ["retry", [key("status"), key("jobs"), key("job", 8), key("log", 8), key("issues")]],
      ["mark done", [key("status"), key("jobs"), key("job", 10), key("issues")]],
      ["save binding", [key("bindings"), key("issues"), key("doctor"), key("status")]],
      ["remove binding", [key("bindings"), key("issues"), key("doctor"), key("status")]],
      ["save settings", [key("settings"), key("status"), key("issues"), key("doctor")]],
      ["api key", [key("status"), key("issues"), key("projects"), key("doctor")]],
      ["log out", [key("status"), key("issues"), key("projects"), key("doctor")]],
      ["reconnect", [key("status"), key("issues"), key("projects"), key("doctor")]],
      ["setup", [key("doctor"), key("controlServers"), key("linearServerNamed")]],
      ["autostart", [key("autostart")]],
    ];
    for (const [label, expected] of expectations) {
      expect(await keysAfterClicking(label, spy), label).toEqual(expect.arrayContaining(expected));
    }
  });

  test("saving the settings is over only once they have been read again, so a page that moves on sees them", async () => {
    // The first-run gate looks at `onboarding_completed`: the guide saves it and goes to the overview.
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    let seen: boolean | undefined;
    function Finish() {
      const client = useQueryClient();
      const settings = useSettings();
      const save = useSaveSettings();
      const finish = () => {
        if (!settings.data) return;
        save.mutate(
          { ...settings.data, onboarding_completed: true },
          { onSuccess: () => (seen = client.getQueryData<Settings>(queryKeys.settings)?.onboarding_completed) },
        );
      };
      return (
        <>
          <p>{settings.data ? "settings read" : "reading"}</p>
          <Act run={finish} />
        </>
      );
    }
    await renderWithProviders(<Finish />, { backend });
    await screen.findByText("settings read");
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    await waitFor(() => expect(seen).toBeDefined());
    expect(seen).toBe(true);
  });

  test("a saved binding is in the list by the time saving is over", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    let seen: string[] | undefined;
    function Bind() {
      const client = useQueryClient();
      const bindings = useBindings();
      const save = useSaveBinding();
      const bind = () => {
        if (!bindings.data) return;
        const binding: Binding = { id: "proj-3", project_id: "proj-3", project_name: "Docs Site", directory: "~/code/docs" };
        save.mutate(binding, { onSuccess: () => (seen = client.getQueryData<Binding[]>(queryKeys.bindings)?.map((b) => b.id)) });
      };
      return (
        <>
          <p>{bindings.data ? "bindings read" : "reading"}</p>
          <Act run={bind} />
        </>
      );
    }
    await renderWithProviders(<Bind />, { backend });
    await screen.findByText("bindings read");
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    await waitFor(() => expect(seen).toBeDefined());
    expect(seen).toEqual(["proj-3"]);
  });

  test("the re-read also reaches data that no page shows right now", async () => {
    // The user looked at the settings and went elsewhere: the cache still holds them, nothing observes them.
    const backend = createMockBackend("demo", { latencyMs: 0 });
    let seen: number | undefined;
    function Save() {
      const client = useQueryClient();
      const save = useSaveSettings();
      const run = async () => {
        const settings = await backend.settings();
        save.mutate(
          { ...settings, poll_interval_secs: 60 },
          { onSuccess: () => (seen = client.getQueryData<Settings>(queryKeys.settings)?.poll_interval_secs) },
        );
      };
      return <Act run={() => void run()} />;
    }
    const { client } = await renderWithProviders(<Save />, { backend });
    await client.fetchQuery({ queryKey: queryKeys.settings, queryFn: () => backend.settings() });
    expect(client.getQueryData<Settings>(queryKeys.settings)?.poll_interval_secs).toBe(30);
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    await waitFor(() => expect(seen).toBeDefined());
    expect(seen).toBe(60);
  });

  test("a binding that is saved is in the cached list even when no page shows the bindings", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    let seen: string[] | undefined;
    function Bind() {
      const client = useQueryClient();
      const save = useSaveBinding();
      const binding: Binding = { id: "proj-3", project_id: "proj-3", project_name: "Docs Site", directory: "~/code/docs" };
      const run = () =>
        save.mutate(binding, { onSuccess: () => (seen = client.getQueryData<Binding[]>(queryKeys.bindings)?.map((b) => b.id)) });
      return <Act run={run} />;
    }
    const { client } = await renderWithProviders(<Bind />, { backend });
    await client.fetchQuery({ queryKey: queryKeys.bindings, queryFn: () => backend.bindings() });
    expect(client.getQueryData<Binding[]>(queryKeys.bindings)).toEqual([]);
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    await waitFor(() => expect(seen).toBeDefined());
    expect(seen).toEqual(["proj-3"]);
  });

  test("a failure says what could not be done in the interface language, with the cleaned line as the reason", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.cancelJob = async () => {
      throw new BackendError("job 12 is done and cannot be stopped\u001b[2J");
    };
    function Cancel() {
      const cancel = useCancelJob();
      return <Act run={() => cancel.mutate(12)} />;
    }
    const { client } = await renderWithProviders(<Cancel />, { backend, lang: "zh-CN" });
    const spy = vi.spyOn(client, "invalidateQueries");
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    expect(await screen.findByText("无法停止这次运行")).toBeInTheDocument();
    expect(screen.getByText("job 12 is done and cannot be stopped")).toBeInTheDocument();
    expect(spy).not.toHaveBeenCalled();
  });

  test("a save that the form reports itself is not told as a toast as well, and the failure stays on the mutation", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.saveBinding = async () => {
      throw new BackendError("disk is full");
    };
    function Bind() {
      const save = useSaveBinding({ quiet: true });
      const binding: Binding = { id: "proj-3", project_id: "proj-3", project_name: "Docs Site", directory: "~/code/docs" };
      return (
        <>
          <Act run={() => save.mutate(binding)} />
          {save.isError && <p>failed: {save.error.message}</p>}
        </>
      );
    }
    await renderWithProviders(<Bind />, { backend });
    const error = vi.spyOn(toast, "error");
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    expect(await screen.findByText("failed: disk is full")).toBeInTheDocument();
    expect(error).not.toHaveBeenCalled();
  });

  test("a settings save that the caller reports itself is not told as a toast as well, and the failure comes back to the caller", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.saveSettings = async () => {
      throw new BackendError("config.toml is read-only");
    };
    function Save({ quiet }: { quiet: boolean }) {
      const save = useSaveSettings({ quiet });
      const [told, setTold] = useState("");
      const run = async () => {
        try {
          await save.mutateAsync(await backend.settings());
        } catch (error) {
          setTold((error as Error).message);
        }
      };
      return (
        <>
          <Act run={() => void run()} />
          {told !== "" && <p>failed: {told}</p>}
        </>
      );
    }
    await renderWithProviders(<Save quiet />, { backend });
    const error = vi.spyOn(toast, "error");
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    expect(await screen.findByText("failed: config.toml is read-only")).toBeInTheDocument();
    expect(error).not.toHaveBeenCalled();
  });

  test("a settings save that is not quiet is told as a toast", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.saveSettings = async () => {
      throw new BackendError("config.toml is read-only");
    };
    function Save() {
      const save = useSaveSettings();
      return <Act run={async () => save.mutate(await backend.settings())} />;
    }
    await renderWithProviders(<Save />, { backend });
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    expect(await screen.findByText("Could not save the settings")).toBeInTheDocument();
    expect(screen.getByText("config.toml is read-only")).toBeInTheDocument();
  });

  test("every action names itself when it fails", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const refuse = async () => {
      throw new BackendError("refused");
    };
    Object.assign(backend, {
      pause: refuse, resume: refuse, pollNow: refuse, cancelJob: refuse, retryJob: refuse, markDone: refuse,
      saveBinding: refuse, removeBinding: refuse, saveSettings: refuse, linearUseApiKey: refuse, linearLogout: refuse,
      linearReconnect: refuse, setupAction: refuse, setAutostart: refuse,
    });
    function Everything() {
      const hooks = {
        pause: usePause(), resume: useResume(), "poll now": usePollNow(), cancel: useCancelJob(), retry: useRetryJob(),
        "mark done": useMarkDone(), "save binding": useSaveBinding(), "remove binding": useRemoveBinding(),
        "save settings": useSaveSettings(), "api key": useLinearApiKey(), "log out": useLinearLogout(),
        reconnect: useLinearReconnect(), setup: useSetupAction(), autostart: useSetAutostart(),
      };
      const run = (name: keyof typeof hooks) => (hooks[name].mutate as (variables?: unknown) => void)(undefined);
      return (
        <>
          {(Object.keys(hooks) as (keyof typeof hooks)[]).map((name) => (
            <Act key={name} label={name} run={() => run(name)} />
          ))}
        </>
      );
    }
    await renderWithProviders(<Everything />, { backend });
    const headlines: [string, string][] = [
      ["pause", "Could not pause monitoring"],
      ["resume", "Could not resume monitoring"],
      ["poll now", "Could not start a check"],
      ["cancel", "Could not stop the run"],
      ["retry", "Could not run it again"],
      ["mark done", "Could not mark it done"],
      ["save binding", "Could not save the binding"],
      ["remove binding", "Could not remove the binding"],
      ["save settings", "Could not save the settings"],
      ["api key", "Could not use the API key"],
      ["log out", "Could not log out of Linear"],
      ["reconnect", "Could not reconnect to Linear"],
      ["setup", "The setup command failed"],
      ["autostart", "Could not change start at login"],
    ];
    const error = vi.spyOn(toast, "error");
    for (const [label, headline] of headlines) {
      error.mockClear();
      await userEvent.click(screen.getByRole("button", { name: label }));
      await waitFor(() => expect(error, label).toHaveBeenCalledTimes(1));
      expect(error.mock.calls[0], label).toEqual([headline, { description: "refused" }]);
    }
  });

  test("a failure the backend answers with as a code shows that code's text and no raw line", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.pause = async () => {
      throw new BackendError("not_a_linear_link");
    };
    function Pause() {
      const pause = usePause();
      return <Act run={() => pause.mutate()} />;
    }
    await renderWithProviders(<Pause />, { backend });
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    expect(await screen.findByText("IssueRelay only opens Linear's own pages")).toBeInTheDocument();
    expect(screen.queryByText("not_a_linear_link")).not.toBeInTheDocument();
  });

  test("the reason is shown as text, never as markup", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    backend.pause = async () => {
      throw new BackendError("<img src=x onerror=alert(1)> <b>bold</b>");
    };
    function Pause() {
      const pause = usePause();
      return <Act run={() => pause.mutate()} />;
    }
    await renderWithProviders(<Pause />, { backend });
    await userEvent.click(screen.getByRole("button", { name: "go" }));
    expect(await screen.findByText("<img src=x onerror=alert(1)> <b>bold</b>")).toBeInTheDocument();
    expect(document.querySelector("img[src=x]")).toBeNull();
    expect(document.querySelector("b")).toBeNull();
  });
});

describe("the Linear login", () => {
  /** A login that stays open until `finish` or `cancel`, as the browser round trip does. */
  function waitingLogin(backend: Backend) {
    let settleLogin: { resolve: () => void; reject: (error: Error) => void } = { resolve: () => {}, reject: () => {} };
    backend.linearLogin = () =>
      new Promise((resolve, reject) => {
        settleLogin = { resolve: () => resolve({ id: "user-1", name: "Jane Doe" }), reject };
      });
    backend.linearCancelLogin = vi.fn(async () => settleLogin.reject(new BackendError("login_cancelled")));
    return { finish: () => settleLogin.resolve() };
  }

  function LoginProbe() {
    const login = useLinearLogin();
    const waiting = useLinearLoginWaiting();
    return (
      <>
        <button type="button" onClick={() => login.mutate()}>
          log in
        </button>
        <button type="button" onClick={() => void login.cancel()}>
          give up
        </button>
        <p>{waiting ? "waiting for the browser" : "not waiting"}</p>
      </>
    );
  }

  test("is waiting from the click until the browser answers, and then refreshes who is logged in", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    const { finish } = waitingLogin(backend);
    const { client } = await renderWithProviders(<LoginProbe />, { backend });
    const spy = vi.spyOn(client, "invalidateQueries");
    expect(screen.getByText("not waiting")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "log in" }));
    expect(await screen.findByText("waiting for the browser")).toBeInTheDocument();
    await act(async () => finish());
    expect(await screen.findByText("not waiting")).toBeInTheDocument();
    for (const expected of [["status"], ["issues"], ["projects"], ["doctor"]]) {
      expect(invalidated(spy)).toContain(JSON.stringify(expected));
    }
  });

  test("cancelling is the user's own doing: the login stops waiting and nothing is said", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    waitingLogin(backend);
    const error = vi.spyOn(toast, "error");
    await renderWithProviders(<LoginProbe />, { backend });
    await userEvent.click(screen.getByRole("button", { name: "log in" }));
    expect(await screen.findByText("waiting for the browser")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "give up" }));
    expect(backend.linearCancelLogin).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("not waiting")).toBeInTheDocument();
    expect(error).not.toHaveBeenCalled();
  });

  test("a login that fails says so", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    backend.linearLogin = async () => {
      throw new BackendError("the authorization page did not answer");
    };
    await renderWithProviders(<LoginProbe />, { backend });
    await userEvent.click(screen.getByRole("button", { name: "log in" }));
    expect(await screen.findByText("Could not log in to Linear")).toBeInTheDocument();
    expect(screen.getByText("the authorization page did not answer")).toBeInTheDocument();
    expect(screen.getByText("not waiting")).toBeInTheDocument();
  });

  test("a cancel that itself fails is reported", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    backend.linearCancelLogin = async () => {
      throw new BackendError("no login is waiting");
    };
    await renderWithProviders(<LoginProbe />, { backend });
    await userEvent.click(screen.getByRole("button", { name: "give up" }));
    expect(await screen.findByText("Could not log in to Linear")).toBeInTheDocument();
    expect(screen.getByText("no login is waiting")).toBeInTheDocument();
  });
});

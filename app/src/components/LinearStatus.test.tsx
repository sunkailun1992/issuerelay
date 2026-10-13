import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { useLinearLogin } from "@/hooks/queries";
import { type Backend, BackendError } from "@/lib/backend";
import { MOCK_NOW, setFixedClock } from "@/lib/clock";
import { createMockBackend } from "@/lib/mock";
import type { LinearSetup, StatusSnapshot, StatusView, Viewer } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { LinearStatus } from "./LinearStatus";

// core takes one change of the login at a time, and a login waits for the browser for up to ten
// minutes: the user must be able to give it up.
test("a login that waits for the browser can be cancelled", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  let cancel = () => {};
  backend.linearLogin = vi.fn(
    () =>
      new Promise<never>((_, reject) => {
        cancel = () => reject(new BackendError("login_cancelled"));
      }),
  );
  backend.linearCancelLogin = vi.fn(async () => cancel());
  await renderWithProviders(<LinearStatus />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Log in to Linear" }));
  expect(await screen.findByText("Finish the authorization in your browser…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Log in to Linear" })).not.toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(backend.linearCancelLogin).toHaveBeenCalledTimes(1);
  expect(await screen.findByRole("button", { name: "Log in to Linear" })).toBeInTheDocument();
  // The user gave it up: no failure is shown.
  expect(screen.queryByText("Could not log in to Linear")).not.toBeInTheDocument();
});

const ESC = String.fromCharCode(27);

/** The demo backend whose status says `linear` (and `patch` of the snapshot). */
async function backendSaying(linear: LinearSetup, patch: Partial<StatusSnapshot> = {}): Promise<Backend> {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const view = await backend.status();
  backend.status = async (): Promise<StatusView> => ({ status: { ...view.status, ...patch }, linear });
  return backend;
}

/** A browser login that waits until `finish` or `cancel` is called. */
function waitingLogin(backend: Backend) {
  let cancel = () => {};
  backend.linearLogin = vi.fn(
    () =>
      new Promise<never>((_, reject) => {
        cancel = () => reject(new BackendError("login_cancelled"));
      }),
  );
  backend.linearCancelLogin = vi.fn(async () => cancel());
}

test("a connected card names the user in an element of its own, and when Linear was last asked", async () => {
  await renderWithProviders(<LinearStatus />);
  const label = await screen.findByText("Connected as");
  const name = screen.getByText("Jane Doe");
  expect(name).not.toBe(label);
  expect(label.contains(name)).toBe(true);
  expect(screen.getByText("Last check Just now")).toBeInTheDocument();
  // Nothing to do when everything works, unless the card is for managing the connection.
  expect(screen.queryAllByRole("button")).toHaveLength(0);
});

test("the time of the last check is worked out again while the card stays on screen", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  try {
    await renderWithProviders(<LinearStatus />);
    expect(await screen.findByText("Last check Just now")).toBeInTheDocument();
    setFixedClock(new Date(MOCK_NOW.getTime() + 5 * 60_000));
    act(() => {
      vi.advanceTimersByTime(30_000);
    });
    expect(screen.getByText("Last check 5 minutes ago")).toBeInTheDocument();
  } finally {
    vi.useRealTimers();
  }
});

test("a connection whose first check has not finished says it has not been checked yet", async () => {
  const backend = await backendSaying({ kind: "connected", method: "oauth" }, { last_poll_at: null });
  await renderWithProviders(<LinearStatus />, { backend });
  expect(await screen.findByText("Not checked yet")).toBeInTheDocument();
});

test("a connection without the user's name yet still says it is connected", async () => {
  const backend = await backendSaying({ kind: "connected", method: "oauth" }, { viewer: null });
  await renderWithProviders(<LinearStatus />, { backend });
  expect(await screen.findByText("Connected")).toBeInTheDocument();
  expect(screen.queryByText("Connected as")).not.toBeInTheDocument();
});

test("the user's name is shown as text, never as markup", async () => {
  const backend = await backendSaying({ kind: "connected", method: "oauth" }, { viewer: { id: "user-1", name: "<b>Jane</b> Doe" } });
  await renderWithProviders(<LinearStatus />, { backend });
  expect(await screen.findByText("<b>Jane</b> Doe")).toBeInTheDocument();
});

test("while the first connection is under way the card says so and offers nothing", async () => {
  await renderWithProviders(<LinearStatus />, { backend: createMockBackend("connecting", { latencyMs: 0 }) });
  expect(await screen.findByText("Connecting to Linear…")).toBeInTheDocument();
  expect(screen.queryAllByRole("button")).toHaveLength(0);
});

test("a first run offers the login, and logging in shows the connection", async () => {
  await renderWithProviders(<LinearStatus />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText("Not logged in to Linear")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Log in to Linear" }));
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Log in to Linear" })).not.toBeInTheDocument();
});

test("a failed connection says why, in the engine's cleaned words, and can be tried again", async () => {
  const backend = await backendSaying({ kind: "connect_failed", message: `Linear did not answer${ESC}[0m` });
  backend.linearReconnect = vi.fn().mockResolvedValue({ kind: "connect_failed", message: "still down" });
  await renderWithProviders(<LinearStatus />, { backend });
  expect(await screen.findByText("Could not connect to Linear")).toBeInTheDocument();
  expect(screen.getByText("Linear did not answer")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(backend.linearReconnect).toHaveBeenCalledTimes(1);
});

test("changed tools of Linear are named and explained, with nothing to press", async () => {
  const backend = await backendSaying({ kind: "interface_changed", message: "Linear no longer has the tool `list_issues`" });
  await renderWithProviders(<LinearStatus />, { backend });
  expect(await screen.findByText("Linear's tools changed: update IssueRelay")).toBeInTheDocument();
  expect(screen.getByText("Linear no longer has the tool `list_issues`")).toBeInTheDocument();
  expect(screen.queryAllByRole("button")).toHaveLength(0);
});

test("the card for managing the connection can authorize again and log out; the plain card cannot", async () => {
  const first = await renderWithProviders(<LinearStatus />);
  await screen.findByText("Jane Doe");
  expect(screen.queryByRole("button", { name: "Authorize again" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Log out" })).not.toBeInTheDocument();
  first.unmount();

  const backend = createMockBackend("demo", { latencyMs: 0 });
  const logout = vi.spyOn(backend, "linearLogout");
  await renderWithProviders(<LinearStatus manage />, { backend });
  expect(await screen.findByRole("button", { name: "Authorize again" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Log out" }));
  expect(logout).toHaveBeenCalledTimes(1);
  expect(await screen.findByRole("button", { name: "Log in to Linear" })).toBeInTheDocument();
});

test("authorizing again waits for the browser like the first login does, and can be cancelled", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  waitingLogin(backend);
  await renderWithProviders(<LinearStatus manage />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Authorize again" }));
  expect(await screen.findByText("Finish the authorization in your browser…")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Authorize again" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Log out" })).not.toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(await screen.findByRole("button", { name: "Authorize again" })).toBeInTheDocument();
  // The connection that was there before is still there.
  expect(screen.getByText("Jane Doe")).toBeInTheDocument();
  expect(screen.queryByText("Could not log in to Linear")).not.toBeInTheDocument();
});

test("a login started elsewhere shows as waiting on every card, and cancelling it reaches the backend", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  waitingLogin(backend);
  function Elsewhere() {
    const login = useLinearLogin();
    return (
      <button type="button" onClick={() => login.mutate()}>
        start elsewhere
      </button>
    );
  }
  await renderWithProviders(
    <>
      <Elsewhere />
      <LinearStatus />
    </>,
    { backend },
  );
  await screen.findByRole("button", { name: "Log in to Linear" });
  await userEvent.click(screen.getByRole("button", { name: "start elsewhere" }));
  expect(await screen.findByText("Finish the authorization in your browser…")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(backend.linearCancelLogin).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(screen.queryByText("Finish the authorization in your browser…")).not.toBeInTheDocument());
});

test("a login the browser answers ends the waiting and shows the connection", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  let finish = () => {};
  const read = backend.linearLogin.bind(backend);
  backend.linearLogin = () =>
    new Promise<Viewer>((resolve, reject) => {
      finish = () => read().then(resolve, reject);
    });
  await renderWithProviders(<LinearStatus />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Log in to Linear" }));
  expect(await screen.findByText("Finish the authorization in your browser…")).toBeInTheDocument();
  finish();
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
  expect(screen.queryByText("Finish the authorization in your browser…")).not.toBeInTheDocument();
});

test("a login that fails is reported, and the card goes back to the login", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  backend.linearLogin = vi.fn().mockRejectedValue(new BackendError("Linear sent back an error"));
  await renderWithProviders(<LinearStatus />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Log in to Linear" }));
  expect(await screen.findByText("Could not log in to Linear")).toBeInTheDocument();
  expect(screen.getByText("Linear sent back an error")).toBeInTheDocument();
  expect(await screen.findByRole("button", { name: "Log in to Linear" })).toBeInTheDocument();
});

test("while the status is read the card is a placeholder", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.status = () => new Promise<StatusView>(() => {});
  await renderWithProviders(<LinearStatus />, { backend });
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
});

test("a status that cannot be read is an error with a retry, not a card on a guess", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.status.bind(backend);
  backend.status = vi.fn().mockRejectedValueOnce(new BackendError("the engine did not answer")).mockImplementation(read);
  await renderWithProviders(<LinearStatus />, { backend });
  expect(await screen.findByText("Could not read IssueRelay's status")).toBeInTheDocument();
  expect(screen.getByText("the engine did not answer")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByText("Jane Doe")).toBeInTheDocument();
});

test("the card reads in the interface language", async () => {
  await renderWithProviders(<LinearStatus />, { lang: "zh-CN" });
  expect(await screen.findByText(/^已连接/)).toBeInTheDocument();
  expect(screen.getByText("Jane Doe")).toBeInTheDocument();
  expect(screen.getByText("上次检查：刚刚")).toBeInTheDocument();
});

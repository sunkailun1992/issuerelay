import { onlineManager } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { initI18nOnce, setLanguage } from "@/i18n";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { JobState, QuitPrompt, StartupState, StatusView } from "@/lib/types";
import App from "./App";

afterEach(async () => {
  document.documentElement.classList.remove("dark");
  onlineManager.setOnline(true);
  // The page is still on screen (the cleanup comes after this), so the change reaches it inside act.
  await act(async () => {
    await setLanguage("en");
  });
  // App routes by the address's fragment and follows the language setting. The address goes last:
  // the router acts on a change of it, and must be gone from the screen by the time it is told.
  window.location.hash = "";
});

test("the panel opens on the overview inside the shell", async () => {
  await initI18nOnce("en");
  render(<App backend={createMockBackend("demo", { latencyMs: 0 })} />);
  expect(await screen.findByRole("heading", { name: "Overview" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Runs" })).toBeInTheDocument();
});

test("any start failure replaces the panel with a readable reason and a Quit button", async () => {
  await initI18nOnce("en");
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.startupState = async () => ({ kind: "failed", code: "store_unavailable", detail: "cannot open state.db" });
  render(<App backend={backend} />);
  expect(await screen.findByText("IssueRelay could not open its run records")).toBeInTheDocument();
  expect(screen.getByText("cannot open state.db")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Quit" })).toBeInTheDocument();
  // The panel is gone, not hidden behind the page.
  expect(screen.queryByRole("link", { name: "Runs" })).not.toBeInTheDocument();
});

test("a failure that happens after the panel opened replaces it too", async () => {
  await initI18nOnce("en");
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let fail: (state: StartupState) => void = () => {};
  backend.onStartupFailed = async (listener) => {
    fail = listener;
    return () => {};
  };
  render(<App backend={backend} />);
  expect(await screen.findByRole("heading", { name: "Overview" })).toBeInTheDocument();
  act(() => fail({ kind: "failed", code: "already_running", detail: "another instance is running" }));
  expect(await screen.findByText(/Another IssueRelay is already running/)).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Overview" })).not.toBeInTheDocument();
});

test("the quit prompt works while the panel is replaced by the start failure", async () => {
  await initI18nOnce("en");
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.startupState = async () => ({ kind: "failed", code: "other", detail: "boom" });
  let ask: (prompt: QuitPrompt) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  render(<App backend={backend} />);
  expect(await screen.findByText("Something stopped IssueRelay from starting")).toBeInTheDocument();
  await act(async () => ask({ running: 1, waiting: false }));
  expect(screen.getByText(/1 run is in progress/)).toBeInTheDocument();
});

test("every page is reachable from the sidebar and has its header", async () => {
  await initI18nOnce("en");
  render(<App backend={createMockBackend("demo", { latencyMs: 0 })} />);
  await screen.findByRole("heading", { name: "Overview" });
  for (const [link, title] of [
    ["Issues", "Issues"],
    ["Runs", "Runs"],
    ["Bindings", "Bindings"],
    ["Environment", "Environment"],
    ["Settings", "Settings"],
    ["Help", "Help"],
    ["Overview", "Overview"],
  ]) {
    await userEvent.click(screen.getByRole("link", { name: link }));
    expect(await screen.findByRole("heading", { name: title }), link).toBeInTheDocument();
  }
});

test("a run's address shows the runs page, and an unknown address goes to the overview", async () => {
  await initI18nOnce("en");
  window.location.hash = "#/runs/12";
  const first = render(<App backend={createMockBackend("demo", { latencyMs: 0 })} />);
  expect(await screen.findByRole("heading", { name: "Runs" })).toBeInTheDocument();
  first.unmount();
  window.location.hash = "#/nowhere/at/all";
  render(<App backend={createMockBackend("demo", { latencyMs: 0 })} />);
  expect(await screen.findByRole("heading", { name: "Overview" })).toBeInTheDocument();
});

test("a first run goes to the guide, which has no sidebar", async () => {
  await initI18nOnce("en");
  render(<App backend={createMockBackend("fresh", { latencyMs: 0 })} />);
  expect(await screen.findByRole("heading", { level: 1, name: "Choose your language" })).toBeInTheDocument();
  expect(window.location.hash).toBe("#/onboarding");
  expect(screen.queryByRole("link", { name: "Runs" })).not.toBeInTheDocument();
  // Wait for what the first step reads, so that it does not arrive after the test.
  expect(await screen.findByRole("radio", { name: "Follow the system" })).toBeChecked();
});

test("the language chosen in the settings is followed once they are read", async () => {
  await initI18nOnce("en");
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.settings.bind(backend);
  backend.settings = async () => ({ ...(await read()), language: "ja" });
  render(<App backend={backend} />);
  expect(await screen.findByRole("link", { name: ja.nav.runs })).toBeInTheDocument();
  expect(document.documentElement.lang).toBe("ja");
});

test("the interface follows the system's dark appearance", async () => {
  await initI18nOnce("en");
  const media = {
    matches: true,
    media: "(prefers-color-scheme: dark)",
    addEventListener: () => {},
    removeEventListener: () => {},
  } as unknown as MediaQueryList;
  vi.spyOn(window, "matchMedia").mockReturnValue(media);
  render(<App backend={createMockBackend("demo", { latencyMs: 0 })} />);
  await screen.findByRole("heading", { name: "Overview" });
  expect(document.documentElement).toHaveClass("dark");
});

test("the panel keeps reading and acting while the system reports it is offline", async () => {
  // Every call is local IPC: "offline" says nothing about it, and must not freeze the panel.
  await initI18nOnce("en");
  onlineManager.setOnline(false);
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const pause = vi.spyOn(backend, "pause");
  render(<App backend={backend} />);
  expect(await screen.findByRole("heading", { name: "Overview" })).toBeInTheDocument();
  expect(await screen.findByText("Version 0.1.0")).toBeInTheDocument();
  await userEvent.click(await screen.findByRole("switch", { name: "Monitoring" }));
  await waitFor(() => expect(pause).toHaveBeenCalledTimes(1));
});

test("a page that cannot read what the backend gave shows the error page, not a blank window", async () => {
  await initI18nOnce("en");
  // React reports an error it hands to a boundary; keep the output of the test run readable.
  vi.spyOn(console, "error").mockImplementation(() => {});
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let ask: (prompt: QuitPrompt) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  // A reply the panel cannot make sense of: what reads the status around the pages (the refresh
  // of the Linear data in the event bridge) fails while it renders, before any page does.
  backend.status = async () => ({}) as StatusView;
  render(<App backend={backend} />);
  expect(await screen.findByRole("heading", { name: en.crash.title })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: en.crash.reload })).toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Runs" })).not.toBeInTheDocument();
  // The quit prompt is outside what failed, and still asks.
  await act(async () => ask({ running: 2, waiting: false }));
  expect(screen.getByText(/2 runs are in progress/)).toBeInTheDocument();
});

/**
 * The demo backend, with run 12 in a state the panel does not know: its page fails while it
 * renders, as a page does on a reply it cannot read. The quit prompt is the backend's to open.
 */
function withFailingRunPage() {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.job.bind(backend);
  backend.job = async (id) => {
    const job = await read(id);
    return job === null ? null : { ...job, state: "unheard_of" as JobState };
  };
  let ask: (prompt: QuitPrompt) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  return { backend, ask: (prompt: QuitPrompt) => act(async () => ask(prompt)) };
}

test("a page that fails leaves the quit prompt and the toasts working", async () => {
  await initI18nOnce("en");
  // React reports an error it hands to a boundary; keep the output of the test run readable.
  vi.spyOn(console, "error").mockImplementation(() => {});
  const { backend, ask } = withFailingRunPage();
  backend.quit = vi.fn().mockRejectedValueOnce(new BackendError("the host is shutting down")).mockResolvedValue(undefined);
  window.location.hash = "#/runs/12";
  render(<App backend={backend} />);
  expect(await screen.findByRole("heading", { name: en.crash.page_title })).toBeInTheDocument();
  // The tray's Quit with a run in progress: the prompt opens over the error page and answers.
  await ask({ running: 1, waiting: false });
  expect(screen.getByText(/1 run is in progress/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  // The answer the app refused is told in a toast, which still has a place to appear.
  expect(await screen.findByText("the host is shutting down")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Stop now" }));
  expect(backend.quit).toHaveBeenLastCalledWith("stop");
});

test("going to another page recovers from a page that failed", async () => {
  await initI18nOnce("en");
  vi.spyOn(console, "error").mockImplementation(() => {});
  const { backend } = withFailingRunPage();
  window.location.hash = "#/runs/12";
  render(<App backend={backend} />);
  expect(await screen.findByRole("heading", { name: en.crash.page_title })).toBeInTheDocument();
  await act(async () => {
    window.location.hash = "#/";
  });
  expect(await screen.findByRole("heading", { name: "Overview" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: en.crash.page_title })).not.toBeInTheDocument();
});

test("a page that fails shows the error inside the shell, and another page from the sidebar recovers", async () => {
  await initI18nOnce("en");
  vi.spyOn(console, "error").mockImplementation(() => {});
  const { backend, ask } = withFailingRunPage();
  window.location.hash = "#/runs/12";
  render(<App backend={backend} />);
  expect(await screen.findByRole("heading", { name: en.crash.page_title })).toBeInTheDocument();
  expect(screen.getByText(en.crash.page_hint)).toBeInTheDocument();
  // The shell is still there, with its sidebar and the way to the other pages.
  expect(screen.getByRole("link", { name: "Runs" })).toBeInTheDocument();
  // The quit prompt still asks.
  await ask({ running: 1, waiting: false });
  expect(screen.getByText(/1 run is in progress/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Keep running" }));
  await userEvent.click(screen.getByRole("link", { name: "Overview" }));
  expect(await screen.findByRole("heading", { name: "Overview" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: en.crash.page_title })).not.toBeInTheDocument();
});

test("closing the quit prompt over a page that failed puts the focus on the error's heading", async () => {
  await initI18nOnce("en");
  vi.spyOn(console, "error").mockImplementation(() => {});
  const { backend, ask } = withFailingRunPage();
  window.location.hash = "#/runs/12";
  render(<App backend={backend} />);
  const heading = await screen.findByRole("heading", { name: en.crash.page_title });
  // The tray's Quit, with nothing in the window focused.
  await ask({ running: 1, waiting: false });
  await userEvent.click(screen.getByRole("button", { name: "Keep running" }));
  await waitFor(() => expect(heading).toHaveFocus());
});

test("closing the quit prompt over the start failure puts the focus on its heading", async () => {
  await initI18nOnce("en");
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.startupState = async () => ({ kind: "failed", code: "other", detail: "boom" });
  let ask: (prompt: QuitPrompt) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  render(<App backend={backend} />);
  const heading = await screen.findByRole("heading", { name: "IssueRelay could not start" });
  await act(async () => ask({ running: 1, waiting: false }));
  await userEvent.click(screen.getByRole("button", { name: "Keep running" }));
  await waitFor(() => expect(heading).toHaveFocus());
});

test("Reload on the error page goes back to the overview, where the page that failed is not", async () => {
  await initI18nOnce("en");
  // Also keeps out jsdom's note that it cannot really reload.
  vi.spyOn(console, "error").mockImplementation(() => {});
  const { backend } = withFailingRunPage();
  window.location.hash = "#/runs/12";
  render(<App backend={backend} />);
  await userEvent.click(await screen.findByRole("button", { name: en.crash.reload }));
  expect(window.location.hash).toBe("#/");
});

test("the region of the toasts is named in the interface language", async () => {
  await initI18nOnce("en");
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.settings.bind(backend);
  backend.settings = async () => ({ ...(await read()), language: "ja" });
  render(<App backend={backend} />);
  expect(await screen.findByRole("link", { name: ja.nav.runs })).toBeInTheDocument();
  // The library adds its keyboard shortcut to the name.
  expect(screen.getByRole("region", { name: new RegExp(`^${ja.shell.notifications} `) })).toBeInTheDocument();
});

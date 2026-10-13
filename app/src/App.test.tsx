import { onlineManager } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { initI18nOnce, setLanguage } from "@/i18n";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { createMockBackend } from "@/lib/mock";
import type { StartupState, StatusView } from "@/lib/types";
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
  let ask: (running: number) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  render(<App backend={backend} />);
  expect(await screen.findByText("Something stopped IssueRelay from starting")).toBeInTheDocument();
  await act(async () => ask(1));
  expect(screen.getByText(/1 run\(s\) are in progress/)).toBeInTheDocument();
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
  // A reply the panel cannot make sense of: the sidebar fails while it renders.
  backend.status = async () => ({}) as StatusView;
  render(<App backend={backend} />);
  expect(await screen.findByRole("heading", { name: en.crash.title })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: en.crash.reload })).toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Runs" })).not.toBeInTheDocument();
});

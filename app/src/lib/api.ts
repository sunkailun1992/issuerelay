import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type Backend, BackendError } from "./backend";
import type { StartupState } from "./types";

export const isTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw new BackendError(typeof error === "string" ? error : String(error));
  }
}

async function on<T>(event: string, listener: (payload: T) => void): Promise<() => void> {
  return listen<T>(event, (e) => listener(e.payload));
}

export const tauriBackend: Backend = {
  appInfo: () => call("app_info"),
  startupState: () => call("startup_state"),
  status: () => call("status"),
  listJobs: (limit) => call("list_jobs", { limit }),
  job: (id) => call("job", { id }),
  jobLog: (id, lines) => call("job_log", { id, lines }),
  issues: () => call("issues"),
  pause: () => call("pause"),
  resume: () => call("resume"),
  pollNow: () => call("poll_now"),
  cancelJob: (id) => call("cancel_job", { id }),
  retryJob: (id) => call("retry_job", { id }),
  markDone: (id) => call("mark_done", { id }),
  bindings: () => call("bindings"),
  saveBinding: (binding) => call("save_binding", { binding }),
  removeBinding: (id) => call("remove_binding", { id }),
  projects: () => call("projects"),
  settings: () => call("settings"),
  saveSettings: (settings) => call("save_settings", { settings }),
  doctor: () => call("doctor"),
  linearLogin: () => call("linear_login"),
  linearCancelLogin: () => call("linear_cancel_login"),
  linearUseApiKey: (key) => call("linear_use_api_key", { key }),
  linearLogout: () => call("linear_logout"),
  linearReconnect: () => call("linear_reconnect"),
  setupAction: (action) => call("setup_action", { action }),
  controlServers: () => call("control_servers"),
  linearServerNamed: (executor) => call("linear_server_named", { executor }),
  openLinear: (url) => call("open_linear", { url }),
  chooseDirectory: () => call("choose_directory"),
  autostartEnabled: () => call("autostart_enabled"),
  setAutostart: (enabled) => call("set_autostart", { enabled }),
  quit: (mode) => call("quit", { mode }),
  requestQuit: () => call("request_quit"),
  onEngineEvent: (listener) => on("engine-event", listener),
  onResync: (listener) => on("engine-resync", () => listener()),
  onQuitRequested: (listener) => on("quit-requested", listener),
  onStartupFailed: (listener) => on<StartupState>("startup-failed", listener),
};

/**
 * `?mock=fresh` in a browser starts the mock as a first run (no login, no bindings);
 * `?mock=connecting` as a start whose first connection to Linear is still under way.
 */
function mockScenario(): "demo" | "fresh" | "connecting" {
  const params = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
  const scenario = params.get("mock");
  return scenario === "fresh" || scenario === "connecting" ? scenario : "demo";
}

/** `?lang=zh-CN` in a browser makes the mock's settings ask for that language (screenshots). */
function mockLanguage(): string | undefined {
  const params = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
  return params.get("lang") ?? undefined;
}

/**
 * The backend the panel uses: the real commands inside Tauri; in a plain browser the mock,
 * loaded with `import()` only then, so the demo data stays out of the app's own start-up code.
 */
export async function loadBackend(): Promise<Backend> {
  if (isTauri()) return tauriBackend;
  const { createMockBackend } = await import("./mock");
  return createMockBackend(mockScenario(), { language: mockLanguage() });
}

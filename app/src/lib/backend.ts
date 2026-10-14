import type {
  AppInfo, Binding, ControlServers, DoctorReport, EngineEvent, ExecutorKind, IssueView, Job, LinearSetup,
  Project, QuitMode, QuitPrompt, Settings, SetupAction, StartupState, StatusView, Viewer,
} from "./types";

/** Errors a command answers with as a code rather than a sentence; the panel translates them (`errors.*`). */
export const ERROR_CODES = ["not_a_linear_link", "login_cancelled"] as const;
export type ErrorCode = (typeof ERROR_CODES)[number];

/** A command failed. `message` is the backend's one cleaned line; `code` is set when it is one of `ERROR_CODES`. */
export class BackendError extends Error {
  readonly code: ErrorCode | null;

  constructor(message: string) {
    super(message);
    this.name = "BackendError";
    this.code = (ERROR_CODES as readonly string[]).includes(message) ? (message as ErrorCode) : null;
  }
}

export interface Backend {
  appInfo(): Promise<AppInfo>;
  startupState(): Promise<StartupState>;
  status(): Promise<StatusView>;
  listJobs(limit: number): Promise<Job[]>;
  job(id: number): Promise<Job | null>;
  jobLog(id: number, lines: number): Promise<string[]>;
  issues(): Promise<IssueView[]>;
  pause(): Promise<void>;
  resume(): Promise<void>;
  pollNow(): Promise<void>;
  cancelJob(id: number): Promise<void>;
  retryJob(id: number): Promise<void>;
  markDone(id: number): Promise<void>;
  bindings(): Promise<Binding[]>;
  saveBinding(binding: Binding): Promise<void>;
  removeBinding(id: string): Promise<boolean>;
  projects(): Promise<Project[]>;
  settings(): Promise<Settings>;
  saveSettings(settings: Settings): Promise<void>;
  doctor(): Promise<DoctorReport>;
  linearLogin(): Promise<Viewer>;
  linearCancelLogin(): Promise<void>;
  linearUseApiKey(key: string): Promise<Viewer>;
  linearLogout(): Promise<void>;
  linearReconnect(): Promise<LinearSetup>;
  setupAction(action: SetupAction): Promise<void>;
  controlServers(): Promise<ControlServers>;
  linearServerNamed(executor: ExecutorKind): Promise<boolean>;
  openLinear(url: string): Promise<void>;
  chooseDirectory(): Promise<string | null>;
  autostartEnabled(): Promise<boolean>;
  setAutostart(enabled: boolean): Promise<void>;
  quit(mode: QuitMode): Promise<void>;
  requestQuit(): Promise<void>;
  onEngineEvent(listener: (event: EngineEvent) => void): Promise<() => void>;
  onResync(listener: () => void): Promise<() => void>;
  onQuitRequested(listener: (prompt: QuitPrompt) => void): Promise<() => void>;
  onStartupFailed(listener: (state: StartupState) => void): Promise<() => void>;
}

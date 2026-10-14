import { afterEach, describe, expect, test, vi } from "vitest";
import capability from "../../src-tauri/capabilities/default.json";

afterEach(() => {
  // Also after a failed assertion: the next test must not think it runs inside Tauri, or at an address with a scenario.
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  window.history.replaceState(null, "", "/");
  vi.doUnmock("@tauri-apps/api/core");
  vi.doUnmock("@tauri-apps/api/event");
});

describe("backend selection", () => {
  test("outside Tauri the panel loads the mock backend", async () => {
    vi.resetModules();
    const { isTauri, loadBackend } = await import("./api");
    expect(isTauri()).toBe(false);
    const backend = await loadBackend();
    expect((await backend.status()).status.viewer?.name).toBe("Jane Doe");
  });

  test("?lang= in the address makes the mock's settings ask for that language, ?mock= picks the scenario", async () => {
    window.history.replaceState(null, "", "/?mock=fresh&lang=zh-CN");
    vi.resetModules();
    const { loadBackend } = await import("./api");
    const settings = await (await loadBackend()).settings();
    expect(settings.language).toBe("zh-CN");
    expect(settings.onboarding_completed).toBe(false);
  });

  test("inside Tauri the address picks nothing: ?mock= and ?lang= are for the browser preview only", async () => {
    window.history.replaceState(null, "", "/?mock=fresh&lang=zh-CN");
    vi.resetModules();
    const invoke = vi.fn().mockResolvedValue({ language: "system" });
    vi.doMock("@tauri-apps/api/core", () => ({ invoke }));
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    const { loadBackend, tauriBackend } = await import("./api");
    const backend = await loadBackend();
    expect(backend).toBe(tauriBackend);
    // The settings come from the app, not from a mock asked for by the address.
    expect(await backend.settings()).toEqual({ language: "system" });
    expect(invoke).toHaveBeenCalledWith("settings", undefined);
  });

  test("?update= in the address makes the mock's app offer an update or report a failed install", async () => {
    window.history.replaceState(null, "", "/?update=available");
    vi.resetModules();
    const { loadBackend } = await import("./api");
    expect(await (await loadBackend()).updateStatus()).toEqual({ kind: "available", version: "0.2.0" });
    window.history.replaceState(null, "", "/?update=install-failed");
    vi.resetModules();
    const failed = await (await (await import("./api")).loadBackend()).updateStatus();
    expect(failed).toMatchObject({ kind: "failed", code: "install_failed" });
    // A name the mock does not know leaves the app up to date, as without the parameter.
    window.history.replaceState(null, "", "/?update=nonsense");
    vi.resetModules();
    expect((await (await (await import("./api")).loadBackend()).updateStatus()).kind).toBe("up_to_date");
  });

  test("inside Tauri ?update= picks nothing either: the status comes from the app", async () => {
    window.history.replaceState(null, "", "/?update=available");
    vi.resetModules();
    const invoke = vi.fn().mockResolvedValue({ kind: "unchecked" });
    vi.doMock("@tauri-apps/api/core", () => ({ invoke }));
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    const { loadBackend } = await import("./api");
    expect(await (await loadBackend()).updateStatus()).toEqual({ kind: "unchecked" });
    expect(invoke).toHaveBeenCalledWith("update_status", undefined);
  });

  test("without ?lang= the mock's settings follow the system", async () => {
    vi.resetModules();
    const { loadBackend } = await import("./api");
    expect((await (await loadBackend()).settings()).language).toBe("system");
  });

  test("command errors become BackendError with the backend's text, and codes are recognised", async () => {
    vi.resetModules();
    const invoke = vi
      .fn()
      .mockRejectedValueOnce("job 7 not found")
      .mockRejectedValueOnce("not_a_linear_link")
      .mockRejectedValueOnce("login_cancelled")
      .mockRejectedValueOnce("update_bad_signature");
    vi.doMock("@tauri-apps/api/core", () => ({ invoke }));
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    const { loadBackend } = await import("./api");
    const { BackendError } = await import("./backend");
    const backend = await loadBackend();
    const failed = await backend.job(7).catch((error: unknown) => error);
    expect(failed).toBeInstanceOf(BackendError);
    expect(failed).toMatchObject({ message: "job 7 not found", code: null });
    const refused = await backend.openLinear("https://evil.example/").catch((error: unknown) => error);
    expect(refused).toMatchObject({ code: "not_a_linear_link" });
    const cancelled = await backend.linearLogin().catch((error: unknown) => error);
    expect(cancelled).toMatchObject({ code: "login_cancelled" });
    const unsigned = await backend.installUpdate().catch((error: unknown) => error);
    expect(unsigned).toMatchObject({ code: "update_bad_signature" });
  });
});

/** Every module of the panel as text, except the tests (they mock `@tauri-apps/api`). */
const SOURCES = import.meta.glob<string>(["/src/**/*.{ts,tsx}", "!/src/**/*.test.{ts,tsx}"], {
  query: "?raw",
  import: "default",
  eager: true,
});

// The panel's window may call only what app/src-tauri/capabilities/default.json grants; the
// Rust test in app/src-tauri/src/lib.rs holds that file to the commands the app registers.
describe("the webview boundary", () => {
  test("only api.ts reaches Tauri, through invoke and listen", () => {
    const reaching = Object.entries(SOURCES).filter(([, source]) => source.includes("@tauri-apps/api"));
    expect(reaching.map(([path]) => path)).toEqual(["/src/lib/api.ts"]);
    const imports = [...reaching[0][1].matchAll(/import\s*\{([^}]*)\}\s*from\s*"@tauri-apps\/api\/([^"]+)"/g)].map(
      ([, names, module]) => `${module}: ${names.trim()}`,
    );
    expect(imports).toEqual(["core: invoke", "event: listen"]);
  });

  test("the panel calls exactly the commands its capability allows, and nothing else of Tauri's but listen", async () => {
    vi.resetModules();
    const invoke = vi.fn().mockResolvedValue(null);
    const listen = vi.fn().mockResolvedValue(() => {});
    vi.doMock("@tauri-apps/api/core", () => ({ invoke }));
    vi.doMock("@tauri-apps/api/event", () => ({ listen }));
    const { tauriBackend } = await import("./api");
    // Every method once; the mocked calls ignore what they are given.
    for (const method of Object.values(tauriBackend)) {
      await (method as (argument: unknown) => Promise<unknown>)(() => {});
    }
    const called = new Set(invoke.mock.calls.map(([command]) => command as string));
    const granted = capability.permissions
      .filter((permission) => permission.startsWith("allow-"))
      .map((permission) => permission.slice("allow-".length).replaceAll("-", "_"));
    expect(called).toEqual(new Set(granted));
    expect(listen).toHaveBeenCalledTimes(5);
    // `listen` and the function it returns need exactly these two; nothing else of Tauri's own.
    expect(capability.permissions.filter((permission) => !permission.startsWith("allow-"))).toEqual([
      "core:event:allow-listen",
      "core:event:allow-unlisten",
    ]);
  });
});

// AGENTS.md and SECURITY_CODING_SPEC.md section 3: text from outside reaches the page only as a React
// text node. Held by this scan the way the Tauri boundary above is, not by review alone.
test("no module of the panel writes HTML", () => {
  const sinks = ["dangerouslySetInnerHTML", "innerHTML", "outerHTML", "insertAdjacentHTML"];
  const writing = Object.entries(SOURCES).flatMap(([path, source]) =>
    sinks.filter((sink) => source.includes(sink)).map((sink) => `${path}: ${sink}`),
  );
  expect(Object.keys(SOURCES).length, "the panel's modules were read").toBeGreaterThan(100);
  expect(writing).toEqual([]);
});

import { afterEach, describe, expect, test, vi } from "vitest";

afterEach(() => {
  // Also after a failed assertion: the next test must not think it runs inside Tauri.
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.doUnmock("@tauri-apps/api/core");
});

describe("backend selection", () => {
  test("outside Tauri the panel loads the mock backend", async () => {
    vi.resetModules();
    const { isTauri, loadBackend } = await import("./api");
    expect(isTauri()).toBe(false);
    const backend = await loadBackend();
    expect((await backend.status()).status.viewer?.name).toBe("Jane Doe");
  });

  test("command errors become BackendError with the backend's text, and codes are recognised", async () => {
    vi.resetModules();
    const invoke = vi
      .fn()
      .mockRejectedValueOnce("job 7 not found")
      .mockRejectedValueOnce("not_a_linear_link")
      .mockRejectedValueOnce("login_cancelled");
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
  });
});

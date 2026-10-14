import { renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { vi } from "vitest";
import { BackendProvider, useBackend } from "./backend-context";
import { createMockBackend } from "./mock";

test("a component without a provider fails loudly instead of using some default backend", () => {
  // React reports the error it is about to throw; keep the test output readable.
  vi.spyOn(console, "error").mockImplementation(() => {});
  expect(() => renderHook(() => useBackend())).toThrow("BackendProvider is missing");
});

test("the provider gives its backend to everything below it", () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const { result } = renderHook(() => useBackend(), {
    wrapper: ({ children }: { children: ReactNode }) => <BackendProvider backend={backend}>{children}</BackendProvider>,
  });
  expect(result.current).toBe(backend);
});

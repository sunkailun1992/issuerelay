import { act, screen } from "@testing-library/react";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import type { StartupState } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { advanceUntil } from "@/test/time";
import { StartupGate } from "./StartupGate";

const gated = (
  <StartupGate>
    <p>the panel</p>
  </StartupGate>
);

afterEach(() => {
  vi.useRealTimers();
});

test("a host that started shows the panel", async () => {
  await renderWithProviders(gated);
  expect(await screen.findByText("the panel")).toBeInTheDocument();
});

test("a failure from before the panel was listening is found by asking", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.startupState = async () => ({ kind: "failed", code: "already_running", detail: "another instance holds the lock" });
  await renderWithProviders(gated, { backend });
  expect(await screen.findByText(/Another IssueRelay is already running/)).toBeInTheDocument();
  expect(screen.getByText("another instance holds the lock")).toBeInTheDocument();
  expect(screen.queryByText("the panel")).not.toBeInTheDocument();
});

test("a failure announced while the panel is showing replaces it", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let announce: (state: StartupState) => void = () => {};
  backend.onStartupFailed = async (listener) => {
    announce = listener;
    return () => {};
  };
  await renderWithProviders(gated, { backend });
  expect(await screen.findByText("the panel")).toBeInTheDocument();
  act(() => announce({ kind: "failed", code: "other", detail: "the store is locked" }));
  expect(await screen.findByText("Something stopped IssueRelay from starting")).toBeInTheDocument();
  expect(screen.getByText("the store is locked")).toBeInTheDocument();
  expect(screen.queryByText("the panel")).not.toBeInTheDocument();
});

test("a start still under way shows the panel and asks again every second until it is settled", async () => {
  // Every timer is the test's: the mock answers from a timer and so does the library's redraw, so
  // time is advanced by hand and nothing is waited for.
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const answers: StartupState[] = [
    { kind: "starting" },
    { kind: "starting" },
    { kind: "failed", code: "store_unavailable", detail: "cannot open state.db" },
  ];
  let asked = 0;
  backend.startupState = vi.fn(async () => answers[Math.min(asked++, answers.length - 1)]);
  const asks = () => vi.mocked(backend.startupState).mock.calls.length;

  await renderWithProviders(gated, { backend });
  expect(asks()).toBe(1);
  expect(screen.getByText("the panel")).toBeInTheDocument();

  expect(await advanceUntil(() => asks() === 2)).toBe(1000);
  expect(screen.getByText("the panel")).toBeInTheDocument();

  const failure = () => screen.queryByText("IssueRelay could not open its run records") !== null;
  expect(await advanceUntil(failure)).toBeLessThan(1200);
  expect(asks()).toBe(3);

  // Settled: nobody asks again.
  await act(() => vi.advanceTimersByTimeAsync(10_000));
  expect(asks()).toBe(3);
});

test("a host that is ready is not asked again", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const ask = vi.spyOn(backend, "startupState");
  await renderWithProviders(gated, { backend });
  expect(ask).toHaveBeenCalledTimes(1);
  await act(() => vi.advanceTimersByTimeAsync(10_000));
  expect(ask).toHaveBeenCalledTimes(1);
});

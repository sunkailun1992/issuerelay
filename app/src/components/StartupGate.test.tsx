import { act, screen } from "@testing-library/react";
import { vi } from "vitest";
import { createMockBackend } from "@/lib/mock";
import type { StartupState } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
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
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const answers: StartupState[] = [
    { kind: "starting" },
    { kind: "starting" },
    { kind: "failed", code: "store_unavailable", detail: "cannot open state.db" },
  ];
  let asked = 0;
  backend.startupState = vi.fn(async () => answers[Math.min(asked++, answers.length - 1)]);
  const settle = () => act(() => new Promise<void>((done) => setTimeout(done, 5)));
  const second = () => act(() => vi.advanceTimersByTimeAsync(1000).then(() => new Promise<void>((done) => setTimeout(done, 5))));

  await renderWithProviders(gated, { backend });
  await settle();
  expect(backend.startupState).toHaveBeenCalledTimes(1);
  expect(screen.getByText("the panel")).toBeInTheDocument();

  await second();
  expect(backend.startupState).toHaveBeenCalledTimes(2);
  expect(screen.getByText("the panel")).toBeInTheDocument();

  await second();
  expect(backend.startupState).toHaveBeenCalledTimes(3);
  expect(screen.getByText("IssueRelay could not open its run records")).toBeInTheDocument();

  // Settled: nobody asks again.
  await second();
  await second();
  expect(backend.startupState).toHaveBeenCalledTimes(3);
});

test("a host that is ready is not asked again", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const ask = vi.spyOn(backend, "startupState");
  await renderWithProviders(gated, { backend });
  await act(() => new Promise<void>((done) => setTimeout(done, 5)));
  await act(() => vi.advanceTimersByTimeAsync(5000));
  expect(ask).toHaveBeenCalledTimes(1);
});

import { waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { vi } from "vitest";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { useJob, useJobs, useStatus } from "@/hooks/queries";
import { createMockBackend } from "@/lib/mock";
import type { EngineEvent } from "@/lib/types";
import { renderWithProviders } from "@/test/render";

test("engine events refresh exactly the data they touch", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let emit: (event: EngineEvent) => void = () => {};
  let resync: () => void = () => {};
  const listen = vi.fn(async (listener: (event: EngineEvent) => void) => {
    emit = listener;
    return () => {};
  });
  backend.onEngineEvent = listen;
  backend.onResync = async (listener) => {
    resync = listener;
    return () => {};
  };
  const { client } = await renderWithProviders(<EngineEventsBridge />, { backend });
  const spy = vi.spyOn(client, "invalidateQueries");
  // Wait until the bridge has really subscribed (`emit` exists from the start, so waiting on it
  // would prove nothing).
  await waitFor(() => expect(listen).toHaveBeenCalledWith(expect.any(Function)));
  const job = (await backend.listJobs(1))[0];
  emit({ kind: "job", job });
  const keys = spy.mock.calls.map(([filters]) => JSON.stringify(filters?.queryKey));
  for (const key of [["status"], ["jobs"], ["job", job.id], ["issues"]]) {
    expect(keys).toContain(JSON.stringify(key));
  }
  spy.mockClear();
  emit({ kind: "config" });
  expect(spy.mock.calls.map(([f]) => JSON.stringify(f?.queryKey))).toEqual(
    expect.arrayContaining([JSON.stringify(["settings"]), JSON.stringify(["bindings"])]),
  );
  spy.mockClear();
  // A Status event re-reads the status, which carries the Linear setup: "connecting" turns into
  // "connected" or "connect_failed" without a reload.
  emit({ kind: "status" });
  expect(spy.mock.calls.map(([f]) => JSON.stringify(f?.queryKey))).toContain(JSON.stringify(["status"]));
  spy.mockClear();
  resync();
  expect(spy).toHaveBeenCalledWith();
});

test("a job event leaves what it does not touch alone", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let emit: (event: EngineEvent) => void = () => {};
  const listen = vi.fn(async (listener: (event: EngineEvent) => void) => {
    emit = listener;
    return () => {};
  });
  backend.onEngineEvent = listen;
  const { client } = await renderWithProviders(<EngineEventsBridge />, { backend });
  const spy = vi.spyOn(client, "invalidateQueries");
  await waitFor(() => expect(listen).toHaveBeenCalled());
  const job = (await backend.listJobs(1))[0];
  emit({ kind: "job", job });
  const keys = spy.mock.calls.map(([f]) => JSON.stringify(f?.queryKey));
  for (const untouched of [["settings"], ["bindings"], ["doctor"], ["projects"], ["appInfo"]]) {
    expect(keys).not.toContain(JSON.stringify(untouched));
  }
  spy.mockClear();
  emit({ kind: "status" });
  expect(spy.mock.calls.map(([f]) => JSON.stringify(f?.queryKey))).toEqual([JSON.stringify(["status"])]);
});

test("a finished job's log is read once more, so its last lines are not lost", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let emit: (event: EngineEvent) => void = () => {};
  const listen = vi.fn(async (listener: (event: EngineEvent) => void) => {
    emit = listener;
    return () => {};
  });
  backend.onEngineEvent = listen;
  const { client } = await renderWithProviders(<EngineEventsBridge />, { backend });
  const spy = vi.spyOn(client, "invalidateQueries");
  await waitFor(() => expect(listen).toHaveBeenCalled());
  const job = (await backend.listJobs(1))[0];
  emit({ kind: "job", job });
  expect(spy.mock.calls.map(([f]) => JSON.stringify(f?.queryKey))).toContain(JSON.stringify(["log", job.id]));
});

test("unmounting the bridge unsubscribes from both event sources", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const stopEvents = vi.fn();
  const stopResync = vi.fn();
  const listenEvents = vi.fn(async () => stopEvents);
  const listenResync = vi.fn(async () => stopResync);
  backend.onEngineEvent = listenEvents;
  backend.onResync = listenResync;
  const { unmount } = await renderWithProviders(<EngineEventsBridge />, { backend });
  await waitFor(() => expect(listenEvents).toHaveBeenCalled());
  await waitFor(() => expect(listenResync).toHaveBeenCalled());
  unmount();
  await waitFor(() => expect(stopEvents).toHaveBeenCalledTimes(1));
  expect(stopResync).toHaveBeenCalledTimes(1);
});

test("a job event makes the lists and pages that show the job read it again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let emit: (event: EngineEvent) => void = () => {};
  const listen = vi.fn(async (listener: (event: EngineEvent) => void) => {
    emit = listener;
    return () => {};
  });
  backend.onEngineEvent = listen;
  function Shows() {
    const jobs = useJobs();
    const job = useJob(8);
    const status = useStatus();
    return (
      <p>
        {jobs.data?.find((entry) => entry.id === 8)?.state} {job.data?.state} {status.data?.status.queued}
      </p>
    );
  }
  const { findByText } = await renderWithProviders(
    <>
      <EngineEventsBridge />
      <Shows />
    </>,
    { backend },
  );
  await findByText("failed failed 1");
  await waitFor(() => expect(listen).toHaveBeenCalled());
  // The run goes again; the app announces the job, and the lists, the run's page and the counts follow.
  await backend.retryJob(8);
  emit({ kind: "job", job: (await backend.job(8))! });
  await findByText("queued queued 2");
});

test("React's double mount in development leaves exactly one subscription", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const live = { events: 0, resync: 0 };
  backend.onEngineEvent = async () => {
    live.events++;
    return () => {
      live.events--;
    };
  };
  backend.onResync = async () => {
    live.resync++;
    return () => {
      live.resync--;
    };
  };
  const { unmount } = await renderWithProviders(
    <StrictMode>
      <EngineEventsBridge />
    </StrictMode>,
    { backend },
  );
  await waitFor(() => expect(live).toEqual({ events: 1, resync: 1 }));
  unmount();
  await waitFor(() => expect(live).toEqual({ events: 0, resync: 0 }));
});

import { act, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { vi } from "vitest";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { useDoctor, useIssues, useJob, useJobs, useProjects, useStatus } from "@/hooks/queries";
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

describe("when the connection to Linear changes", () => {
  /** What needs a connection to Linear, as three lines of text: the answer or why there is none. */
  function Dependent() {
    const issues = useIssues();
    const projects = useProjects();
    const doctor = useDoctor();
    return (
      <>
        <p>issues: {issues.error ? issues.error.message : (issues.data?.length ?? "reading")}</p>
        <p>projects: {projects.error ? projects.error.message : (projects.data?.length ?? "reading")}</p>
        <p>doctor: {doctor.data ? (doctor.data.viewer?.name ?? doctor.data.linear_error) : "reading"}</p>
      </>
    );
  }

  test("what depends on it is read again, so a page does not keep the answer it got before the connection", async () => {
    const backend = createMockBackend("connecting", { latencyMs: 0 });
    await renderWithProviders(
      <>
        <EngineEventsBridge />
        <Dependent />
      </>,
      { backend },
    );
    expect(await screen.findByText("issues: still connecting to Linear")).toBeInTheDocument();
    expect(await screen.findByText("projects: still connecting to Linear")).toBeInTheDocument();
    expect(await screen.findByText("doctor: still connecting to Linear")).toBeInTheDocument();
    // The first connection ends; the app announces a changed setup, the status says "connected".
    await act(() => backend.linearReconnect());
    expect(await screen.findByText("issues: 9")).toBeInTheDocument();
    expect(await screen.findByText("projects: 3")).toBeInTheDocument();
    expect(await screen.findByText("doctor: Jane Doe")).toBeInTheDocument();
  });

  test("a connection that ends is noticed the same way", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    await renderWithProviders(
      <>
        <EngineEventsBridge />
        <Dependent />
      </>,
      { backend },
    );
    expect(await screen.findByText("issues: 9")).toBeInTheDocument();
    await act(() => backend.linearLogout());
    expect(await screen.findByText("issues: not connected to Linear: log in first")).toBeInTheDocument();
    expect(await screen.findByText("projects: not connected to Linear: log in first")).toBeInTheDocument();
  });

  test("React's double run of an effect in development is no change of the connection", async () => {
    // The bridge mounts when the status is already there, so that its first effect run knows the connection.
    const backend = createMockBackend("demo", { latencyMs: 0 });
    function WhenStatusIsRead() {
      return useStatus().data ? <EngineEventsBridge /> : null;
    }
    const { client } = await renderWithProviders(
      <StrictMode>
        <WhenStatusIsRead />
      </StrictMode>,
      { backend },
    );
    const spy = vi.spyOn(client, "invalidateQueries");
    await waitFor(() => expect(client.getQueryData(["status"])).toBeDefined());
    await act(async () => {});
    expect(spy.mock.calls.map(([filters]) => JSON.stringify(filters?.queryKey))).not.toContain(JSON.stringify(["issues"]));
  });

  test("an event that leaves the connection as it is reads nothing again", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    // Every read of the status is a different one (the queue count), but the connection stays.
    const view = await backend.status();
    let reads = 0;
    backend.status = async () => ({ ...view, status: { ...view.status, queued: ++reads } });
    const issues = vi.spyOn(backend, "issues");
    const projects = vi.spyOn(backend, "projects");
    const doctor = vi.spyOn(backend, "doctor");
    function Queue() {
      return <p>queued: {useStatus().data?.status.queued}</p>;
    }
    await renderWithProviders(
      <>
        <EngineEventsBridge />
        <Dependent />
        <Queue />
      </>,
      { backend },
    );
    await screen.findByText("issues: 9");
    await screen.findByText("queued: 1");
    // Another status is read after the event; once it shows, a changed connection would already have been noticed.
    await act(() => backend.pollNow());
    await screen.findByText("queued: 2");
    expect(issues).toHaveBeenCalledTimes(1);
    expect(projects).toHaveBeenCalledTimes(1);
    expect(doctor).toHaveBeenCalledTimes(1);
  });
});

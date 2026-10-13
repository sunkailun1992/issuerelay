import { describe, expect, test } from "vitest";
import { createMockBackend, pickedUp } from "./mock";
import { DEMO_ISSUES, DEMO_JOBS } from "./mock-fixtures";
import type { EngineEvent } from "./types";

describe("mock backend", () => {
  test("the demo scenario has a job in every state", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const states = new Set((await backend.listJobs(100)).map((job) => job.state));
    for (const state of ["waiting_binding", "queued", "running", "awaiting_acceptance", "needs_info", "done", "failed", "canceled"]) {
      expect(states.has(state as never), state).toBe(true);
    }
  });

  test("marking a job done changes it and announces the change", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const events: EngineEvent[] = [];
    await backend.onEngineEvent((event) => events.push(event));
    const waiting = (await backend.listJobs(100)).find((job) => job.state === "awaiting_acceptance")!;
    await backend.markDone(waiting.id);
    expect((await backend.job(waiting.id))?.state).toBe("done");
    expect(events).toContainEqual({ kind: "job", job: expect.objectContaining({ id: waiting.id, state: "done" }) });
  });

  test("marking a job that is not awaiting acceptance fails like the engine does", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const running = (await backend.listJobs(100)).find((job) => job.state === "running")!;
    await expect(backend.markDone(running.id)).rejects.toThrow(`job ${running.id} is not awaiting acceptance`);
  });

  test("only the newest run of an issue can run again, as the engine says", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    await expect(backend.retryJob(2)).rejects.toThrow("job 2 is not the latest run of DEMO-12; retry that one instead");
    await backend.retryJob(8);
    expect((await backend.job(8))?.state).toBe("queued");
  });

  test("a held job says when it is tried again", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    expect((await backend.status()).status.held).toEqual([{ job_id: 11, retry_at: "2026-10-12T09:02:00Z" }]);
  });

  test("waiting issues carry a reason code, not a sentence", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const waiting = (await backend.issues()).find((view) => view.issue.identifier === "DEMO-5")!;
    expect(waiting.status).toEqual({ kind: "waiting", reason: { code: "project_not_bound" } });
    expect(waiting.job?.state).toBe("waiting_binding");
  });

  test("an eligible issue is picked up unless its newest run ended and Linear has not changed it since", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const issueIsPickedUp = async (identifier: string) => {
      const view = (await backend.issues()).find((candidate) => candidate.issue.identifier === identifier)!;
      if (view.status.kind !== "eligible") throw new Error(`${identifier} is ${view.status.kind}`);
      return view.status.picked_up;
    };
    expect(await issueIsPickedUp("DEMO-3")).toBe(true); // no run at all
    expect(await issueIsPickedUp("DEMO-11")).toBe(true); // queued: in hand
    expect(await issueIsPickedUp("DEMO-6")).toBe(false); // stopped by the user, the issue unchanged since
    // Stopping the queued run leaves it as DEMO-6 is: the engine does not start it again by itself.
    await backend.cancelJob(11);
    expect(await issueIsPickedUp("DEMO-11")).toBe(false);
    // Running a stopped run again puts it back in hand.
    await backend.retryJob(11);
    expect(await issueIsPickedUp("DEMO-11")).toBe(true);
    await backend.retryJob(6);
    expect(await issueIsPickedUp("DEMO-6")).toBe(true);
  });

  test("an ended run counts from what it verified, or else from what it discovered", () => {
    const issue = { ...DEMO_ISSUES[0].issue, updated_at: "2026-10-12T08:00:00Z" };
    const ended = { ...DEMO_JOBS[0], state: "failed" as const, discovered_updated_at: "2026-10-12T07:00:00Z", verified_updated_at: null };
    // Linear changed the issue after the run found it: the next check picks it up again.
    expect(pickedUp(issue, ended)).toBe(true);
    // The run read the issue again after that change (the AI's own edits, say): nothing new.
    expect(pickedUp(issue, { ...ended, verified_updated_at: "2026-10-12T08:00:00Z" })).toBe(false);
    expect(pickedUp(issue, { ...ended, verified_updated_at: "2026-10-12T08:00:01Z" })).toBe(false);
    // A change after what the run verified brings it back.
    expect(pickedUp({ ...issue, updated_at: "2026-10-12T08:00:02Z" }, { ...ended, verified_updated_at: "2026-10-12T08:00:01Z" })).toBe(true);
  });

  test("the verdicts written into the fixtures are what the mock works out for them at the start", () => {
    // The mock works the verdict of an eligible issue out again from its newest run (so that stopping
    // or running again changes it); the flag in the fixture is the one it starts with.
    for (const { issue, status } of DEMO_ISSUES) {
      if (status.kind !== "eligible") continue;
      const newest = DEMO_JOBS.filter((job) => job.issue_uuid === issue.uuid).sort((a, b) => b.id - a.id)[0] ?? null;
      expect(status.picked_up, issue.identifier).toBe(pickedUp(issue, newest));
    }
  });

  test("a binding whose folder is not absolute is refused with the engine's message", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const binding = { id: "proj-3", project_id: "proj-3", project_name: "Docs Site", directory: "code/docs" };
    await expect(backend.saveBinding(binding)).rejects.toThrow('bindings[2].directory must be an absolute path: "code/docs"');
    await backend.saveBinding({ ...binding, directory: "~/code/docs" });
    expect((await backend.bindings()).map((b) => b.id)).toEqual(["proj-1", "proj-2", "proj-3"]);
  });

  test("the fresh scenario is a first run", async () => {
    const backend = createMockBackend("fresh", { latencyMs: 0 });
    expect((await backend.status()).linear.kind).toBe("not_configured");
    expect(await backend.bindings()).toEqual([]);
    expect((await backend.settings()).onboarding_completed).toBe(false);
    await expect(backend.issues()).rejects.toThrow("not connected to Linear: log in first");
    await backend.linearLogin();
    expect((await backend.status()).linear).toEqual({ kind: "connected", method: "oauth" });
    expect((await backend.doctor()).viewer?.name).toBe("Jane Doe");
  });

  test("settings keep the pause switch, as the engine does", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    await backend.pause();
    const settings = await backend.settings();
    await backend.saveSettings({ ...settings, paused: false, poll_interval_secs: 60 });
    expect((await backend.settings()).paused).toBe(true);
    expect((await backend.settings()).poll_interval_secs).toBe(60);
  });

  test("the app info reports a system language, as Rust does", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    expect((await backend.appInfo()).system_locale).toBe("en-US");
    expect(await backend.startupState()).toEqual({ kind: "ready" });
  });

  test("a start still connecting to Linear is ready, and connects on reconnect", async () => {
    const backend = createMockBackend("connecting", { latencyMs: 0 });
    expect(await backend.startupState()).toEqual({ kind: "ready" });
    expect((await backend.status()).linear).toEqual({ kind: "connecting" });
    await expect(backend.issues()).rejects.toThrow("still connecting to Linear");
    const events: EngineEvent[] = [];
    await backend.onEngineEvent((event) => events.push(event));
    await backend.linearReconnect();
    expect((await backend.status()).linear).toEqual({ kind: "connected", method: "oauth" });
    expect(events).toContainEqual({ kind: "status" });
  });

  test("links outside linear.app are refused, as the app refuses them", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    await backend.openLinear("https://mcp.linear.app/authorize");
    for (const url of ["https://linear.app@evil.example/", "https://linear.app.evil.example/", "http://linear.app/"]) {
      await expect(backend.openLinear(url), url).rejects.toMatchObject({ code: "not_a_linear_link" });
    }
  });

  test("control servers report one state per executor", async () => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    expect(await backend.controlServers()).toEqual({ claude: "not_registered", codex: "not_registered" });
    await backend.setupAction({ kind: "register_control_server", executor: "claude" });
    expect((await backend.controlServers()).claude).toBe("registered");
  });
});

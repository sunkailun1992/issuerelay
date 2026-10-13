import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { Job, JobState } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { JobActions } from "./JobActions";

async function jobIn(state: JobState) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.listJobs(100)).find((j) => j.state === state)!;
  return { backend, job };
}

test.each([
  ["running", ["Stop"]],
  ["queued", ["Stop"]],
  ["waiting_binding", ["Stop"]],
  ["failed", ["Run again"]],
  ["canceled", ["Run again"]],
  ["needs_info", ["Run again"]],
  ["awaiting_acceptance", ["Mark done"]],
  ["done", []],
] as [JobState, string[]][])("a %s job offers %j", async (state, expected) => {
  const { backend, job } = await jobIn(state);
  await renderWithProviders(<JobActions job={job} />, { backend });
  for (const name of ["Stop", "Run again", "Mark done"]) {
    const button = screen.queryByRole("button", { name });
    expect(Boolean(button), name).toBe(expected.includes(name));
  }
});

test("only the newest run of an issue offers Run again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const older = (await backend.job(2))!; // DEMO-12's earlier failed run; DEMO-12 runs again now
  await renderWithProviders(<JobActions job={older} />, { backend });
  await waitFor(() => expect(screen.queryByRole("button", { name: "Run again" })).not.toBeInTheDocument());
});

test("an issue without a run only offers to open it in Linear", async () => {
  await renderWithProviders(<JobActions job={null} url="https://linear.app/demo/issue/DEMO-4" />);
  expect(screen.getByRole("button", { name: "Open in Linear" })).toBeInTheDocument();
  for (const name of ["Stop", "Run again", "Mark done"]) {
    expect(screen.queryByRole("button", { name })).not.toBeInTheDocument();
  }
});

test("mark done asks first and names the only write to Linear", async () => {
  const { backend, job } = await jobIn("awaiting_acceptance");
  const markDone = vi.spyOn(backend, "markDone");
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Mark done" }));
  expect(screen.getByText(new RegExp(`${job.issue_identifier}.*Done`))).toBeInTheDocument();
  expect(markDone).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(markDone).toHaveBeenCalledWith(job.id);
});

test("a failed action shows a translated headline and the backend's reason", async () => {
  const { backend, job } = await jobIn("failed");
  backend.retryJob = vi.fn().mockRejectedValue(new BackendError("job 8 is not the latest run of DEMO-8; retry that one instead"));
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Run again" }));
  expect(await screen.findByText("Could not run it again")).toBeInTheDocument();
  expect(screen.getByText(/not the latest run of DEMO-8/)).toBeInTheDocument();
});

test("only Linear links are offered for opening", async () => {
  const { backend, job } = await jobIn("done");
  const open = vi.spyOn(backend, "openLinear");
  await renderWithProviders(<JobActions job={job} url="https://linear.app/demo/issue/DEMO-7" />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Open in Linear" }));
  expect(open).toHaveBeenCalledWith("https://linear.app/demo/issue/DEMO-7");
});

/** Lets what is already under way (answers with no delay) finish. */
const settle = () => act(() => new Promise<void>((done) => setTimeout(done, 10)));

test("stop and run again act on the run beside them", async () => {
  const running = await jobIn("running");
  const cancel = vi.spyOn(running.backend, "cancelJob");
  const first = await renderWithProviders(<JobActions job={running.job} />, { backend: running.backend });
  await userEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(cancel).toHaveBeenCalledWith(running.job.id);
  first.unmount();

  const failed = await jobIn("failed");
  const retry = vi.spyOn(failed.backend, "retryJob");
  await renderWithProviders(<JobActions job={failed.job} />, { backend: failed.backend });
  await userEvent.click(screen.getByRole("button", { name: "Run again" }));
  expect(retry).toHaveBeenCalledWith(failed.job.id);
});

test("an action that is under way cannot be started a second time", async () => {
  const { backend, job } = await jobIn("running");
  backend.cancelJob = vi.fn(() => new Promise<void>(() => {}));
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(backend.cancelJob).toHaveBeenCalledTimes(1);
});

test("a run that is being run again cannot be asked for a second time", async () => {
  const { backend, job } = await jobIn("failed");
  backend.retryJob = vi.fn(() => new Promise<void>(() => {}));
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Run again" }));
  expect(screen.getByRole("button", { name: "Run again" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Run again" }));
  expect(backend.retryJob).toHaveBeenCalledTimes(1);
});

test("while a run is being marked done the question cannot be asked again", async () => {
  const { backend, job } = await jobIn("awaiting_acceptance");
  backend.markDone = vi.fn(() => new Promise<void>(() => {}));
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Mark done" }));
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(screen.getByRole("button", { name: "Mark done" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Mark done" }));
  expect(screen.queryByRole("button", { name: "Confirm" })).not.toBeInTheDocument();
  expect(backend.markDone).toHaveBeenCalledTimes(1);
});

test("the question names the done status the settings configure", async () => {
  const { backend, job } = await jobIn("awaiting_acceptance");
  const settings = await backend.settings();
  backend.settings = async () => ({ ...settings, trigger: { ...settings.trigger, done: "Shipped" } });
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Mark done" }));
  expect(await screen.findByText(new RegExp(`${job.issue_identifier}.*Shipped`))).toBeInTheDocument();
});

test("cancelling the question marks nothing done", async () => {
  const { backend, job } = await jobIn("awaiting_acceptance");
  const markDone = vi.spyOn(backend, "markDone");
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Mark done" }));
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(markDone).not.toHaveBeenCalled();
  expect(screen.queryByText(/only change IssueRelay makes in Linear/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Mark done" })).toBeInTheDocument();
});

test("a mark done the engine refuses is reported with its reason", async () => {
  const { backend, job } = await jobIn("awaiting_acceptance");
  backend.markDone = vi.fn().mockRejectedValue(new BackendError("job 10 is not awaiting acceptance"));
  await renderWithProviders(<JobActions job={job} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Mark done" }));
  await userEvent.click(screen.getByRole("button", { name: "Confirm" }));
  expect(await screen.findByText("Could not mark it done")).toBeInTheDocument();
  expect(screen.getByText("job 10 is not awaiting acceptance")).toBeInTheDocument();
});

test("until the list of runs is read, Run again is offered as usual", async () => {
  const { backend, job } = await jobIn("failed");
  backend.listJobs = () => new Promise<Job[]>(() => {});
  await renderWithProviders(<JobActions job={job} />, { backend });
  await settle();
  expect(screen.getByRole("button", { name: "Run again" })).toBeInTheDocument();
});

test("a list of runs that cannot be read does not take Run again away", async () => {
  const { backend, job } = await jobIn("failed");
  backend.listJobs = vi.fn().mockRejectedValue(new BackendError("the engine did not answer"));
  await renderWithProviders(<JobActions job={job} />, { backend });
  await waitFor(() => expect(backend.listJobs).toHaveBeenCalled());
  await settle();
  expect(screen.getByRole("button", { name: "Run again" })).toBeInTheDocument();
});

test("a run keeps its own actions beside Open in Linear", async () => {
  const { backend, job } = await jobIn("failed");
  await renderWithProviders(<JobActions job={job} url="https://linear.app/demo/issue/DEMO-8" />, { backend });
  expect(screen.getByRole("button", { name: "Run again" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Open in Linear" })).toBeInTheDocument();
});

test("without a link a finished run has nothing to offer", async () => {
  const { backend, job } = await jobIn("done");
  await renderWithProviders(<JobActions job={job} />, { backend });
  expect(screen.queryAllByRole("button")).toHaveLength(0);
});

test("a link the app refuses to open is reported with the translated reason", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.openLinear = vi.fn().mockRejectedValue(new BackendError("not_a_linear_link"));
  await renderWithProviders(<JobActions url="https://example.com/not-linear" />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Open in Linear" }));
  expect(await screen.findByText("IssueRelay only opens Linear's own pages")).toBeInTheDocument();
  expect(screen.queryByText("not_a_linear_link")).not.toBeInTheDocument();
});

test("a link that cannot be opened for another reason says so, with the reason", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.openLinear = vi.fn().mockRejectedValue(new BackendError("no browser is available"));
  await renderWithProviders(<JobActions url="https://linear.app/demo/issue/DEMO-4" />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Open in Linear" }));
  expect(await screen.findByText("Could not open Linear")).toBeInTheDocument();
  expect(screen.getByText("no browser is available")).toBeInTheDocument();
});

test("the compact form keeps the name of each button, with a short Open in Linear", async () => {
  const { backend, job } = await jobIn("awaiting_acceptance");
  await renderWithProviders(<JobActions job={job} url="https://linear.app/demo/issue/DEMO-10" compact />, { backend });
  // An icon with the name on it, no text beside it; the other buttons keep their words.
  expect(screen.getByRole("button", { name: "Open in Linear" }).textContent).toBe("");
  expect(screen.getByRole("button", { name: "Mark done" })).toHaveTextContent("Mark done");
});

test("the buttons read in the interface language", async () => {
  const { backend, job } = await jobIn("failed");
  await renderWithProviders(<JobActions job={job} url="https://linear.app/demo/issue/DEMO-8" />, { backend, lang: "zh-CN" });
  expect(screen.getByRole("button", { name: "重新运行" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "在 Linear 中打开" })).toBeInTheDocument();
});

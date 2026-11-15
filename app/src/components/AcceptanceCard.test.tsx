import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { setLanguage } from "@/i18n";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { AcceptanceCard } from "./AcceptanceCard";

const EXPIRED = "This preview has expired. Reload the preview and review it before confirming again. This action has not been executed.";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function review(seconds = 2) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const job = (await backend.listJobs(100)).find((candidate) => candidate.id === 10)!;
  const prepare = backend.prepareControlAction;
  vi.spyOn(backend, "prepareControlAction").mockImplementation(async (action) => {
    const preview = await prepare(action);
    return {
      ...preview, expires_in_seconds: seconds,
      acceptance: preview.acceptance === null ? null : {
        ...preview.acceptance, delivery_fingerprint: "demo-unchanged-delivery-fingerprint", brief_revision: 4,
      },
    };
  });
  const commit = vi.spyOn(backend, "commitControlAction");
  vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
  const rendered = await renderWithProviders(<AcceptanceCard job={job} />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Start independent verification" }));
  const dialog = within(screen.getByRole("dialog", { name: "Start independent verification" }));
  await userEvent.type(dialog.getByRole("textbox", { name: "Delivery to verify" }), "build-demo-3");
  await userEvent.click(dialog.getByRole("button", { name: "Review action" }));
  await waitFor(() => expect(dialog.getByRole("button", { name: "Confirm action" })).toBeEnabled());
  return { ...rendered, backend, commit, dialog };
}

test("an expired preview explains that nothing ran, and only a fresh review and user confirmation execute it", async () => {
  const view = await review();
  expect(view.dialog.getByText("Preview expires in 2 seconds.")).toBeInTheDocument();
  try {
    await act(() => vi.advanceTimersByTime(1000));
    expect(view.dialog.getByText("Preview expires in 1 second.")).toBeInTheDocument();
    await act(() => vi.advanceTimersByTime(1000));
    expect(view.dialog.getByRole("alert")).toHaveTextContent(EXPIRED);
    const confirm = view.dialog.getByRole("button", { name: "Confirm action" });
    expect(confirm).toBeDisabled();
    await userEvent.click(confirm);
    expect(view.commit).not.toHaveBeenCalled();
    await userEvent.click(view.dialog.getByRole("button", { name: "Reload preview" }));
    expect(await view.dialog.findByText("Preview expires in 2 seconds.")).toBeInTheDocument();
    expect(view.dialog.queryByText(EXPIRED)).not.toBeInTheDocument();
    expect(view.dialog.getByText("demo-unchanged-delivery-fingerprint")).toBeInTheDocument();
    const reads = await Promise.all(vi.mocked(view.backend.prepareControlAction).mock.results.map((result) => result.value));
    expect(reads).toHaveLength(2);
    expect(reads[0].acceptance).toEqual(reads[1].acceptance);
    expect(reads[0].expires_in_seconds).toBe(reads[1].expires_in_seconds);
    expect(reads[0].token).not.toBe(reads[1].token);
    expect(confirm).toBeEnabled();
    expect(view.commit).not.toHaveBeenCalled();
    await userEvent.click(confirm);
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(view.commit).toHaveBeenCalledTimes(1);
    expect((await view.backend.listJobs(100)).filter((job) => job.mode === "verify")).toHaveLength(1);
  } finally {
    view.unmount();
    vi.useRealTimers();
  }
});

test("confirmation checks elapsed time even when the timer has not fired after a clock jump", async () => {
  const view = await review();
  try {
    vi.setSystemTime(Date.now() + 2000);
    await userEvent.click(view.dialog.getByRole("button", { name: "Confirm action" }));
    expect(view.commit).not.toHaveBeenCalled();
    expect(view.dialog.getByRole("alert")).toHaveTextContent(EXPIRED);
    expect(view.dialog.getByRole("button", { name: "Confirm action" })).toBeDisabled();
  } finally {
    view.unmount();
    vi.useRealTimers();
  }
});

test("a backend expiry is readable and reloading clears the failure without confirming", async () => {
  const view = await review(180);
  view.commit.mockRejectedValueOnce(new BackendError("action preview expired"));
  await userEvent.click(view.dialog.getByRole("button", { name: "Confirm action" }));
  expect(await view.dialog.findByText(EXPIRED)).toBeInTheDocument();
  expect(view.dialog.queryByText("Could not approve dispatch")).not.toBeInTheDocument();
  expect(view.dialog.getByRole("button", { name: "Confirm action" })).toBeDisabled();
  await userEvent.click(view.dialog.getByRole("button", { name: "Reload preview" }));
  await waitFor(() => expect(view.dialog.getByRole("button", { name: "Confirm action" })).toBeEnabled());
  expect(view.dialog.queryByText(EXPIRED)).not.toBeInTheDocument();
  expect(view.commit).toHaveBeenCalledTimes(1);
});

test("a submitted action stays busy across expiry without claiming it was not executed", async () => {
  const view = await review();
  let finish = () => {};
  // Use the mock backend's actual mutation after releasing the controlled response.
  view.commit.mockRestore();
  const realCommit = view.backend.commitControlAction;
  const pending = vi.spyOn(view.backend, "commitControlAction").mockImplementationOnce((token) => new Promise((resolve, reject) => {
    finish = () => { void realCommit(token).then(resolve, reject); };
  }));
  await userEvent.click(view.dialog.getByRole("button", { name: "Confirm action" }));
  try {
    await act(() => vi.advanceTimersByTime(3000));
    expect(view.dialog.queryByText(EXPIRED)).not.toBeInTheDocument();
    expect(view.dialog.getByRole("button", { name: "Confirm action" })).toBeDisabled();
    expect(view.dialog.getByRole("button", { name: "Reload preview" })).toBeDisabled();
    expect(view.dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await userEvent.click(view.dialog.getByRole("button", { name: "Confirm action" }));
    expect(pending).toHaveBeenCalledTimes(1);
    await act(async () => finish());
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  } finally {
    view.unmount();
    vi.useRealTimers();
  }
});

test("the open preview follows language changes before and after expiry", async () => {
  const view = await review();
  await act(() => setLanguage("zh-CN"));
  expect(view.dialog.getByText("预览将在 2 秒后过期。")).toBeInTheDocument();
  try {
    await act(() => vi.advanceTimersByTime(2000));
    expect(view.dialog.getByRole("alert")).toHaveTextContent("预览已过期，请重新读取预览并核对后再确认。本次操作尚未执行。");
    await act(() => setLanguage("en"));
    expect(view.dialog.getByRole("alert")).toHaveTextContent(EXPIRED);
    expect(view.dialog.getByRole("button", { name: "Confirm action" })).toBeDisabled();
    for (const [lang, message] of [
      ["zh-TW", "預覽已過期，請重新讀取預覽並核對後再確認。本次操作尚未執行。"],
      ["ja", "このプレビューの有効期限が切れました。プレビューを再読み込みして内容を確認してから、もう一度確定してください。この操作はまだ実行されていません。"],
      ["ko", "이 미리보기가 만료되었습니다. 미리보기를 다시 불러와 내용을 검토한 후 다시 확인하세요. 이 작업은 아직 실행되지 않았습니다."],
    ] as const) {
      await act(() => setLanguage(lang));
      expect(view.dialog.getByRole("alert")).toHaveTextContent(message);
    }
  } finally {
    view.unmount();
    vi.useRealTimers();
  }
});

test("returning a failed verification for repair uses the same expiry guard", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const initial = await backend.prepareControlAction({
    action: "start_verification", job_id: 10, executor: "codex", delivery: "build-demo-3",
  });
  const result = await backend.commitControlAction(initial.token);
  const job = { ...(await backend.job(result.job_id!))!, state: "failed" as const };
  const readHandoff = backend.getHandoff;
  backend.getHandoff = async (id) => {
    const handoff = await readHandoff(id);
    return { ...handoff, acceptance: { ...handoff.acceptance!, verification_status: "failed" as const } };
  };
  backend.prepareControlAction = vi.fn(async (action) => ({
    token: "demo-repair-preview", action, issue_identifier: job.issue_identifier,
    job_id: job.id, current_state: job.state, expires_in_seconds: 2,
    acceptance: {
      source_job_id: 10, delivery: "build-demo-3", reason: "Repeat the synthetic failing check.",
      executor: null, directory: null, model: null, effort: null, permission: null, lean: null,
    },
  }));
  const commit = vi.spyOn(backend, "commitControlAction");
  vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
  await renderWithProviders(<AcceptanceCard job={job} />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Return for repair" }));
  const dialog = within(screen.getByRole("dialog", { name: "Return for repair" }));
  await userEvent.type(dialog.getByRole("textbox", { name: "Repair findings and reproduction" }), "Repeat the synthetic failing check.");
  await userEvent.click(dialog.getByRole("button", { name: "Review action" }));
  expect(await dialog.findByText("Preview expires in 2 seconds.")).toBeInTheDocument();
  await act(() => vi.advanceTimersByTime(2000));
  expect(dialog.getByRole("alert")).toHaveTextContent(EXPIRED);
  expect(dialog.getByRole("button", { name: "Confirm action" })).toBeDisabled();
  expect(dialog.getByRole("button", { name: "Reload preview" })).toBeEnabled();
  expect(commit).not.toHaveBeenCalled();
});

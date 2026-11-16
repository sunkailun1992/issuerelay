import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import i18n, { type Lang, setLanguage } from "@/i18n";
import { queryKeys } from "@/hooks/queries";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { AcceptanceLinks, DispatchPreview } from "@/lib/workflow-types";
import { renderWithProviders } from "@/test/render";
import { AcceptanceCard } from "./AcceptanceCard";
import { PageHeader } from "./PageHeader";

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

test.each([
  ["en", "Return for diagnosis or repair", "Blockers and diagnostic steps"],
  ["zh-CN", "退回诊断或修复", "阻塞原因与诊断步骤"],
  ["zh-TW", "退回診斷或修復", "阻礙原因與診斷步驟"],
  ["ja", "診断または修正のために差し戻す", "阻害要因と診断手順"],
  ["ko", "진단 또는 수정을 위해 반려", "차단 원인과 진단 단계"],
] as const)("a blocked verification returns only after explicit review without changing its conclusion (%s)", async (lang, label, findings) => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const initial = await backend.prepareControlAction({
    action: "start_verification", job_id: 10, executor: "codex", delivery: "build-demo-3",
  });
  const verificationId = (await backend.commitControlAction(initial.token)).job_id!;
  await backend.cancelJob(verificationId);
  // Record a synthetic finding: a stop with no report alone must not permit a return.
  (await backend.getHandoff(verificationId)).acceptance!.verification_evidence = ["The synthetic prerequisite was unavailable."];
  const job = (await backend.job(verificationId))!;
  const jobCount = (await backend.listJobs(100)).length;
  const prepare = vi.spyOn(backend, "prepareControlAction");
  const commit = vi.spyOn(backend, "commitControlAction");
  await renderWithProviders(<AcceptanceCard job={job} />, { backend, lang });
  await userEvent.click(await screen.findByRole("button", { name: label }));
  const dialog = within(screen.getByRole("dialog", { name: label }));
  expect(dialog.getByRole("button", { name: i18n.t("acceptance.action_confirm") })).toBeDisabled();
  const reason = "Diagnose the missing synthetic prerequisite before repeating the check.";
  await userEvent.type(dialog.getByRole("textbox", { name: findings }), reason);
  await userEvent.click(dialog.getByRole("button", { name: i18n.t("acceptance.action_preview") }));
  const confirm = dialog.getByRole("button", { name: i18n.t("acceptance.action_confirm") });
  await waitFor(() => expect(confirm).toBeEnabled());
  expect(prepare).toHaveBeenCalledExactlyOnceWith({ action: "return_for_repair", job_id: verificationId, reason });
  expect(dialog.getByText(reason, { selector: "dd" })).toBeInTheDocument();
  expect(commit).not.toHaveBeenCalled();
  expect((await backend.getHandoff(verificationId)).acceptance).toMatchObject({
    verification_status: "blocked", returned_for_repair: false,
  });
  await userEvent.click(confirm);
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(commit).toHaveBeenCalledTimes(1);
  for (const id of [10, verificationId]) {
    expect((await backend.getHandoff(id)).acceptance).toMatchObject({
      verification_status: "blocked", returned_for_repair: true, repair_reason: reason,
    });
  }
  expect((await backend.listJobs(100)).length).toBe(jobCount);
});

test.each(["passed", "running", "queued", "waiting_binding", null] as const)(
  "a verification with result %s has no return action", async (status) => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const source = (await backend.job(10))!;
    const read = backend.getHandoff;
    backend.getHandoff = async (id) => ({
      ...(await read(source.id)), job_id: id, acceptance: {
        development_job_id: source.id, repair_of_job_id: null, repair_job_id: null,
        verification_job_id: 13, delivery: "build-demo-3", verification_status: status,
        returned_for_repair: false, repair_reason: null,
      },
    });
    const view = await renderWithProviders(<AcceptanceCard job={{ ...source, id: 13, mode: "verify", state: "failed" }} />, { backend });
    expect(await screen.findByRole("link", { name: "Development run #10" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Return for/ })).not.toBeInTheDocument();
    view.unmount();
  },
);

test.each(["waiting_binding", "queued", "running"] as const)(
  "an active verifier cannot return a stale blocked result (%s)", async (state) => {
    const backend = createMockBackend("demo", { latencyMs: 0 });
    const source = (await backend.job(10))!;
    const read = backend.getHandoff;
    backend.getHandoff = async (id) => ({
      ...(await read(source.id)), job_id: id, acceptance: {
        development_job_id: source.id, repair_of_job_id: null, repair_job_id: null,
        verification_job_id: 13, delivery: "build-demo-3", verification_status: "blocked",
        verification_evidence: ["The synthetic prerequisite was unavailable."],
        returned_for_repair: false, repair_reason: null,
      },
    });
    const view = await renderWithProviders(<AcceptanceCard job={{ ...source, id: 13, mode: "verify", state }} />, { backend });
    expect(await screen.findByRole("link", { name: "Development run #10" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Return for/ })).not.toBeInTheDocument();
    view.unmount();
  },
);

test("a superseded blocked verification has no return action", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const start = { action: "start_verification", job_id: 10, executor: "codex", delivery: "build-demo-3" } as const;
  const first = await backend.prepareControlAction(start);
  const oldId = (await backend.commitControlAction(first.token)).job_id!;
  await backend.cancelJob(oldId);
  (await backend.getHandoff(oldId)).acceptance!.verification_evidence = ["The synthetic prerequisite was unavailable."];
  const oldJob = (await backend.job(oldId))!;
  const next = await backend.prepareControlAction(start);
  const newId = (await backend.commitControlAction(next.token)).job_id!;
  expect(newId).toBeGreaterThan(oldId);
  await renderWithProviders(<AcceptanceCard job={oldJob} />, { backend });
  expect(await screen.findByRole("link", { name: "Development run #10" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Return for diagnosis or repair" })).not.toBeInTheDocument();
});

test("a stopped verification with no report cannot return a fallback blocked result", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const initial = await backend.prepareControlAction({
    action: "start_verification", job_id: 10, executor: "codex", delivery: "build-demo-3",
  });
  const id = (await backend.commitControlAction(initial.token)).job_id!;
  await backend.cancelJob(id);
  const job = (await backend.job(id))!;
  expect(job.report).toBeNull();
  expect((await backend.getHandoff(id)).acceptance).toMatchObject({ verification_status: "blocked" });
  await renderWithProviders(<AcceptanceCard job={job} />, { backend });
  expect(await screen.findByRole("link", { name: "Development run #10" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Return for/ })).not.toBeInTheDocument();
});


async function returnedRepairReview(lang: Lang = "en") {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const source = (await backend.job(10))!;
  const job = { ...source, id: 13, mode: "verify" as const, state: "failed" as const };
  const repair = {
    ...source, id: 14, state: "waiting_binding" as const,
    started_at: null, finished_at: null, reason: "waiting for dispatch review",
  };
  let links: AcceptanceLinks = {
    development_job_id: source.id, repair_of_job_id: null, repair_job_id: null,
    verification_job_id: job.id, delivery: "build-demo-3", verification_status: "failed",
    returned_for_repair: true, repair_reason: "Repeat the synthetic failing check.",
  };
  const handoff = await backend.getHandoff(source.id);
  backend.getHandoff = vi.fn(async () => ({ ...handoff, job_id: job.id, acceptance: links }));
  const initialJobs = await backend.listJobs(100);
  backend.listJobs = vi.fn(async () => links.repair_job_id === null ? initialJobs : [repair, ...initialJobs]);
  const preview: DispatchPreview = {
    token: "demo-repair-dispatch-1", issue_uuid: job.issue_uuid, issue_identifier: job.issue_identifier,
    title: job.title, mode: "implement", executor: "codex", directory: "~/code/demo-app",
    model: "gpt-6.1-sol", effort: "ultra",
    context: {
      revision: 4, origin_executor: "codex", preferred_executor: "codex",
      request: "Fix the synthetic validation failure.", conversation_history: "The synthetic verification did not pass.",
      scope: "Repair validation in the demo project.", acceptance: ["The synthetic failing check passes."], source_refs: [],
    },
  };
  const readPreview = vi.spyOn(backend, "getDispatchPreview").mockResolvedValue(preview);
  const approve = vi.spyOn(backend, "approveDispatch").mockResolvedValue(undefined);
  const view = await renderWithProviders(
    <><PageHeader title="Synthetic verification" /><AcceptanceCard job={job} /></>,
    { backend, lang },
  );
  await userEvent.click(await screen.findByRole("button", { name: i18n.t("dispatch.preview") }));
  const dialogElement = screen.getByRole("dialog", { name: i18n.t("dispatch.title") });
  const dialog = within(dialogElement);
  await waitFor(() => expect(dialog.getByRole("button", { name: i18n.t("dispatch.confirm") })).toBeEnabled());
  const pollRepair = async () => {
    links = { ...links, repair_job_id: repair.id };
    await act(async () => {
      await Promise.all([
        view.client.invalidateQueries({ queryKey: queryKeys.handoff(job.id) }),
        view.client.invalidateQueries({ queryKey: queryKeys.jobs }),
      ]);
    });
    await screen.findByText(i18n.t("acceptance.repair_run", { id: repair.id }));
  };
  return { ...view, job, preview, readPreview, approve, dialog, dialogElement, pollRepair };
}

test.each<Lang>(["en", "zh-CN", "zh-TW", "ja", "ko"])(
  "a returned verification keeps its open dispatch review when polling creates a waiting repair (%s)", async (lang) => {
    const view = await returnedRepairReview(lang);
    await view.pollRepair();
    expect(screen.getByRole("dialog", { name: i18n.t("dispatch.title") })).toBe(view.dialogElement);
    expect(view.dialog.getByText("Fix the synthetic validation failure.")).toBeInTheDocument();
    expect(view.dialog.getByText("gpt-6.1-sol")).toBeInTheDocument();
    expect(view.dialog.getByRole("button", { name: i18n.t("dispatch.confirm") })).toBeEnabled();
    expect(view.readPreview).toHaveBeenCalledTimes(1);
    expect(view.approve).not.toHaveBeenCalled();
    await userEvent.click(view.dialog.getByRole("button", { name: i18n.t("common.cancel") }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("heading", { name: "Synthetic verification", level: 1 })).toHaveFocus());
    expect(screen.queryByRole("button", { name: i18n.t("dispatch.preview") })).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: i18n.t("acceptance.repair_run", { id: 14 }) })).toHaveAttribute("href", "/runs/14");
    expect(view.approve).not.toHaveBeenCalled();
  },
);

test("closing a repair dispatch review returns focus to its entry while that entry remains visible", async () => {
  const view = await returnedRepairReview();
  await userEvent.click(view.dialog.getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Preview" })).toHaveFocus());
  expect(view.approve).not.toHaveBeenCalled();
});

test("a retained repair review still reports a rejected stale token and requires a fresh explicit confirmation", async () => {
  const view = await returnedRepairReview();
  view.approve.mockRejectedValueOnce(new BackendError("the dispatch preview changed; read it again"));
  const fresh = {
    ...view.preview, token: "demo-repair-dispatch-2",
    context: { ...view.preview.context!, revision: 5, request: "Review the updated synthetic repair scope." },
  };
  view.readPreview.mockResolvedValue(fresh);
  await view.pollRepair();
  await userEvent.click(view.dialog.getByRole("button", { name: "Confirm dispatch" }));
  expect(await view.dialog.findByText("Could not approve dispatch")).toBeInTheDocument();
  expect(view.dialog.getByText("the dispatch preview changed; read it again")).toBeInTheDocument();
  expect(view.approve).toHaveBeenCalledExactlyOnceWith(view.job.issue_identifier, view.preview.token);
  expect(view.dialog.getByRole("button", { name: "Confirm dispatch" })).toBeDisabled();
  await userEvent.click(view.dialog.getByRole("button", { name: "Reload preview" }));
  expect(await view.dialog.findByText("Review the updated synthetic repair scope.")).toBeInTheDocument();
  expect(view.dialog.queryByText("Could not approve dispatch")).not.toBeInTheDocument();
  expect(view.approve).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(view.dialog.getByRole("button", { name: "Confirm dispatch" })).toBeEnabled());
  await userEvent.click(view.dialog.getByRole("button", { name: "Confirm dispatch" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(view.approve).toHaveBeenNthCalledWith(2, view.job.issue_identifier, fresh.token);
});


test("polling cannot dismiss an in-flight repair approval or enable another submission", async () => {
  const view = await returnedRepairReview();
  let finish = () => {};
  view.approve.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
  await userEvent.click(view.dialog.getByRole("button", { name: "Confirm dispatch" }));
  await view.pollRepair();
  expect(screen.getByRole("dialog", { name: "Review dispatch" })).toBe(view.dialogElement);
  const confirm = view.dialog.getByRole("button", { name: "Confirm dispatch" });
  expect(confirm).toBeDisabled();
  expect(view.dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await userEvent.keyboard("{Escape}");
  expect(view.dialogElement).toBeInTheDocument();
  await userEvent.click(confirm);
  expect(view.approve).toHaveBeenCalledExactlyOnceWith(view.job.issue_identifier, view.preview.token);
  await act(async () => finish());
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});

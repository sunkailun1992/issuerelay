import { Copy, Info } from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Field, Hint } from "@/components/FormField";
import { ErrorState, LoadingState } from "@/components/states";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useClaimHandoff, useHandoff, useRecordArchive, useReleaseHandoff } from "@/hooks/queries";
import { useCopyText } from "@/hooks/useCopyText";
import { useLang } from "@/hooks/useLang";
import { cleanDisplayLines } from "@/lib/clean";
import { formatDateTime } from "@/lib/format";
import { isActive, isNeedsInfoImplementation } from "@/lib/runs";
import type { ExecutorKind, Job } from "@/lib/types";
import type { Handoff, RunMemory, WikiArchive } from "@/lib/workflow-types";

/** A fact recorded for the handoff. Its name comes from the interface; its value is text. */
function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="whitespace-pre-wrap wrap-anywhere">{children}</dd>
    </div>
  );
}

function TextFact({ label, value }: { label: string; value: string | null }) {
  const text = value === null ? "" : cleanDisplayLines(value).trim();
  return text === "" ? null : <Fact label={label}>{text}</Fact>;
}

function visibleItems(values: string[]) {
  return [...new Set(values.map((value) => cleanDisplayLines(value).trim()).filter(Boolean))];
}

function ListFact({ label, values }: { label: string; values: string[] }) {
  const items = visibleItems(values);
  if (items.length === 0) return null;
  return (
    <Fact label={label}>
      <ul className="list-disc space-y-0.5 ps-4">
        {items.map((item) => <li key={item}>{item}</li>)}
      </ul>
    </Fact>
  );
}

export function InformationRequested({ blockers }: { blockers: string[] }) {
  const { t } = useTranslation();
  const items = visibleItems(blockers);
  return (
    <section aria-label={t("handoff.information_requested")} className="space-y-1 text-sm">
      <h3 className="font-medium">{t("handoff.information_requested")}</h3>
      {items.length > 0
        ? <ul className="list-disc space-y-0.5 break-words ps-4">{items.map((item) => <li key={item}>{item}</li>)}</ul>
        : <p className="text-muted-foreground">{t("handoff.information_requested_fallback")}</p>}
    </section>
  );
}

/** What interrupted a run and, when the executor reported it, when its quota resets. */
function Interruption({ interruption }: { interruption: NonNullable<RunMemory["interruption"]> }) {
  const { t } = useTranslation();
  const lang = useLang();
  return (
    <Alert role="status">
      <Info aria-hidden />
      <AlertTitle>{t("handoff.interruption.quota_exhausted")}</AlertTitle>
      {interruption.reset_at !== null && (
        <AlertDescription>
          <dl><Fact label={t("handoff.reset_at")}>{formatDateTime(interruption.reset_at, lang)}</Fact></dl>
        </AlertDescription>
      )}
    </Alert>
  );
}

function HandoffFacts({ context, memory, showBlockers }: Pick<Handoff, "context" | "memory"> & { showBlockers: boolean }) {
  const { t } = useTranslation();
  const lang = useLang();
  if (context === null && memory === null) return <p className="text-sm text-muted-foreground">{t("handoff.empty")}</p>;
  return (
    <>
      {memory?.interruption != null && <Interruption interruption={memory.interruption} />}
      <dl className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,12rem),1fr))] gap-x-6 gap-y-3 text-sm">
        {context !== null && (
          <Fact label={t("handoff.origin")}>
            {context.origin_executor === null ? t("common.unknown") : t(`executor.${context.origin_executor}`)}
          </Fact>
        )}
        {context?.preferred_executor != null && (
          <Fact label={t("handoff.preferred")}>{t(`executor.${context.preferred_executor}`)}</Fact>
        )}
        {memory?.decision != null && (
          <Fact label={t("handoff.selection")}>
            {t(`executor.${memory.decision.executor}`)} · {t(`handoff.source.${memory.decision.source}`)}
          </Fact>
        )}
        {context !== null && (
          <>
            <TextFact label={t("handoff.request")} value={context.request} />
            <TextFact label={t("dispatch.conversation_history")} value={context.conversation_history ?? null} />
            <TextFact label={t("handoff.scope")} value={context.scope} />
            <ListFact label={t("handoff.acceptance")} values={context.acceptance} />
            <ListFact label={t("handoff.sources")} values={context.source_refs} />
          </>
        )}
        {memory?.checkpoint != null && (
          <>
            <TextFact label={t("handoff.checkpoint")} value={memory.checkpoint.summary} />
            <ListFact label={t("handoff.completed")} values={memory.checkpoint.done} />
            <ListFact label={t("handoff.next")} values={memory.checkpoint.next} />
            <ListFact label={t("handoff.verification")} values={memory.checkpoint.verification} />
            {showBlockers && <ListFact label={t("handoff.blockers")} values={memory.checkpoint.blockers} />}
          </>
        )}
        {memory?.git != null && (
          <>
            <TextFact label={t("handoff.git_head")} value={memory.git.head} />
            <TextFact label={t("handoff.git_branch")} value={memory.git.branch} />
            <ListFact label={t("handoff.git_changed")} values={memory.git.changed} />
          </>
        )}
        {memory !== null && <Fact label={t("handoff.updated_at")}>{formatDateTime(memory.updated_at, lang)}</Fact>}
      </dl>
    </>
  );
}

/** The choice starts empty every time the handoff becomes available. */
function Claim({ id }: { id: number }) {
  const { t } = useTranslation();
  const selectId = useId();
  const [executor, setExecutor] = useState<ExecutorKind | "">("");
  const claim = useClaimHandoff(id);
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-end gap-2">
        <Field label={t("handoff.executor")} htmlFor={selectId}>
          <Select value={executor} disabled={claim.isPending} onValueChange={(value) => setExecutor(value as ExecutorKind)}>
            <SelectTrigger id={selectId} className="w-full" aria-describedby={`${selectId}-hint`}>
              <SelectValue placeholder={t("handoff.choose_executor")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="codex">{t("executor.codex")}</SelectItem>
              <SelectItem value="claude">{t("executor.claude")}</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Button disabled={executor === "" || claim.isPending} onClick={() => executor !== "" && claim.mutate(executor)}>
          {t("handoff.claim")}
        </Button>
      </div>
      <Hint id={`${selectId}-hint`}>{t("handoff.claim_hint")}</Hint>
    </div>
  );
}

function Release({ id, executor, claimId }: { id: number; executor: ExecutorKind; claimId: string | null }) {
  const { t } = useTranslation();
  const release = useReleaseHandoff(id);
  return (
    <div className="flex flex-wrap items-center gap-2">
      <p role="status" className="text-sm">{t("handoff.claimed", { executor: t(`executor.${executor}`) })}</p>
      {claimId !== null && (
        <Button variant="outline" disabled={release.isPending} onClick={() => release.mutate(claimId)}>
          {t("handoff.release")}
        </Button>
      )}
    </div>
  );
}

function HandoffActions({ job, handoff }: { job: Job; handoff: Handoff }) {
  const { t } = useTranslation();
  const copy = useCopyText();
  if (job.mode === "verify") return null;
  const hasPrompt = job.state !== "local_complete" && handoff.prompt.trim() !== "";
  const hasReview = handoff.review_prompt.trim() !== "";
  return (
    <>
      {handoff.claimed_by !== null ? (
        <Release id={job.id} executor={handoff.claimed_by} claimId={handoff.claim_id} />
      ) : job.state === "done" ? (
        <p className="text-sm text-muted-foreground">{t("handoff.done")}</p>
      ) : handoff.active || isActive(job) ? (
        <p className="text-sm text-muted-foreground">{t("handoff.active")}</p>
      ) : hasPrompt && (
        <Claim id={job.id} />
      )}
      {(hasPrompt || hasReview) && (
        <div className="flex flex-wrap gap-2">
          {hasPrompt && (
            <Button variant="outline" onClick={() => copy(handoff.prompt)}>
              <Copy aria-hidden />
              {t("handoff.copy")}
            </Button>
          )}
          {hasReview && (
            <Button variant="outline" onClick={() => copy(handoff.review_prompt)}>
              <Copy aria-hidden />
              {t("handoff.copy_review")}
            </Button>
          )}
        </div>
      )}
    </>
  );
}

function ArchiveReceiptForm({ jobId, canReplace, close }: { jobId: number; canReplace: boolean; close: () => void }) {
  const { t } = useTranslation();
  const id = useId();
  const [page, setPage] = useState("");
  const [commit, setCommit] = useState("");
  const save = useRecordArchive(jobId);
  const submit = () => {
    if (page.trim() === "" || save.isPending) return;
    save.mutate({ page: page.trim(), commit: commit.trim() || null }, { onSuccess: close });
  };
  const keepOpenWhileSaving = (event: Event) => {
    if (save.isPending) event.preventDefault();
  };
  return (
    <DialogContent showCloseButton={false} onEscapeKeyDown={keepOpenWhileSaving} onInteractOutside={keepOpenWhileSaving}>
      <DialogHeader>
        <DialogTitle>{t(canReplace ? "acceptance.archive_replace" : "acceptance.archive")}</DialogTitle>
        <DialogDescription>{t("acceptance.archive_description")}</DialogDescription>
      </DialogHeader>
      <form className="space-y-3" onSubmit={(event) => { event.preventDefault(); submit(); }}>
        <div className="space-y-2">
          <Label htmlFor={`${id}-page`}>{t("handoff.archive.page")}</Label>
          <Input id={`${id}-page`} value={page} disabled={save.isPending} onChange={(event) => setPage(event.target.value)} />
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${id}-commit`}>{t("handoff.archive.commit")}</Label>
          <Input id={`${id}-commit`} aria-describedby={`${id}-commit-hint`} value={commit} disabled={save.isPending} onChange={(event) => setCommit(event.target.value)} />
          <Hint id={`${id}-commit-hint`}>{t("acceptance.archive_commit_hint")}</Hint>
        </div>
        {save.isError && <ErrorState error={save.error} headline="acceptance.archive_write_failed" />}
        <DialogFooter>
          <DialogClose asChild><Button type="button" variant="outline" disabled={save.isPending}>{t("common.cancel")}</Button></DialogClose>
          <Button type="submit" disabled={page.trim() === "" || save.isPending}>{t("acceptance.archive_confirm")}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  );
}

function WikiArchiveDetails({ jobId, archive, canRecord }: { jobId: number; archive: WikiArchive; canRecord: boolean }) {
  const { t } = useTranslation();
  const titleId = useId();
  const copy = useCopyText();
  const lang = useLang();
  const [open, setOpen] = useState(false);
  return (
    <section aria-labelledby={titleId} className="flex flex-col items-start gap-3 border-t pt-4">
      <h3 id={titleId} className="font-medium">{t("handoff.archive.title")}</h3>
      <dl className="grid w-full grid-cols-[repeat(auto-fit,minmax(min(100%,12rem),1fr))] gap-x-6 gap-y-3 text-sm">
        <TextFact label={t("handoff.archive.root")} value={archive.root} />
        {archive.receipt !== null && (
          <>
            <TextFact label={t("handoff.archive.page")} value={archive.receipt.page} />
            <TextFact label={t("handoff.archive.commit")} value={archive.receipt.commit} />
            <Fact label={t("handoff.archive.recorded_at")}>{formatDateTime(archive.receipt.recorded_at, lang)}</Fact>
          </>
        )}
      </dl>
      {archive.receipt === null && <p className="text-sm text-muted-foreground">{t("handoff.archive.empty")}</p>}
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" onClick={() => copy(archive.prompt)}>
          <Copy aria-hidden />
          {t("handoff.archive.copy")}
        </Button>
        {canRecord && (
          <Dialog open={open} onOpenChange={setOpen}>
            <DialogTrigger asChild>
              <Button variant="outline">{t(archive.receipt === null ? "acceptance.archive" : "acceptance.archive_replace")}</Button>
            </DialogTrigger>
            {open && <ArchiveReceiptForm jobId={jobId} canReplace={archive.receipt !== null} close={() => setOpen(false)} />}
          </Dialog>
        )}
      </div>
    </section>
  );
}

/** The run's handoff can be read and retried separately from the run and its log. */
export function HandoffCard({ job }: { job: Job }) {
  const { t } = useTranslation();
  const titleId = useId();
  const handoff = useHandoff(job.id);
  const needsInfo = isNeedsInfoImplementation(job);
  return (
    <section aria-labelledby={titleId}>
      <Card size="sm">
        <CardContent className="gap-4">
          <h2 id={titleId} className="text-base font-medium">{t("handoff.title")}</h2>
          {handoff.isError && !handoff.isFetching ? (
            <ErrorState error={handoff.error} headline="handoff.read_failed" onRetry={() => void handoff.refetch()} />
          ) : handoff.data === undefined ? (
            <LoadingState rows={1} />
          ) : (
            <>
              {needsInfo && <InformationRequested blockers={handoff.data.memory?.checkpoint?.blockers ?? []} />}
              <HandoffFacts context={handoff.data.context} memory={handoff.data.memory} showBlockers={!needsInfo} />
              <HandoffActions job={job} handoff={handoff.data} />
              {job.started_at !== null && job.finished_at !== null && (
                handoff.data.archive == null ? (
                  <p className="text-sm text-muted-foreground">{t("acceptance.archive_no_destination")}</p>
                ) : (
                  <WikiArchiveDetails jobId={job.id} archive={handoff.data.archive} canRecord={handoff.data.claimed_by === null} />
                )
              )}
            </>
          )}
        </CardContent>
      </Card>
    </section>
  );
}

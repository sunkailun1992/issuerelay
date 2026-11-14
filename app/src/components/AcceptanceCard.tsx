import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link, useNavigate } from "react-router";
import { DispatchReviewButton, PreviewContext, PreviewFact } from "@/components/DispatchReviewButton";
import { ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Textarea } from "@/components/ui/textarea";
import { useAcceptancePreview, useCommitAcceptance, useHandoff, useJobs } from "@/hooks/queries";
import { useLang } from "@/hooks/useLang";
import { cleanDisplayLines, cleanDisplayText } from "@/lib/clean";
import { formatDateTime } from "@/lib/format";
import type { ExecutorKind, Job } from "@/lib/types";
import type { AcceptanceAction, AcceptanceActionPreview, AcceptanceLinks } from "@/lib/workflow-types";

function ActionFacts({ preview, details }: {
  preview: AcceptanceActionPreview;
  details: NonNullable<AcceptanceActionPreview["acceptance"]>;
}) {
  const { t } = useTranslation();
  const lang = useLang();
  const fingerprint = cleanDisplayText(details.delivery_fingerprint ?? "");
  return (
    <dl className="grid grid-cols-2 gap-3">
      <PreviewFact label={t("runs.field.issue")}>{cleanDisplayText(preview.issue_identifier)}</PreviewFact>
      <PreviewFact label={t("continuation.source_run")}>{t("acceptance.development_run", { id: details.source_job_id })}</PreviewFact>
      {details.delivery !== null && <PreviewFact label={t("acceptance.delivery")}>{cleanDisplayLines(details.delivery)}</PreviewFact>}
      {fingerprint !== "" && (
        <PreviewFact label={t("acceptance.delivery_fingerprint")}>
          <code className="block truncate font-mono text-xs select-text" title={fingerprint}>{fingerprint}</code>
        </PreviewFact>
      )}
      {details.linear_updated_at !== undefined && (
        <PreviewFact label={t("acceptance.linear_updated_at")}>
          <time dateTime={details.linear_updated_at} title={details.linear_updated_at}>{formatDateTime(details.linear_updated_at, lang)}</time>
        </PreviewFact>
      )}
      {details.brief_revision !== undefined && (
        <PreviewFact label={t("acceptance.brief_revision")}>{details.brief_revision ?? "—"}</PreviewFact>
      )}
      {details.reason !== null && <PreviewFact label={t("acceptance.findings")}>{cleanDisplayLines(details.reason)}</PreviewFact>}
      {details.context == null && details.scope && <PreviewFact label={t("acceptance.scope")}>{cleanDisplayLines(details.scope)}</PreviewFact>}
      {details.executor !== null && <PreviewFact label={t("runs.field.executor")}>{t(`executor.${details.executor}`)}</PreviewFact>}
      {details.directory !== null && <PreviewFact label={t("acceptance.original_directory")}>{cleanDisplayText(details.directory)}</PreviewFact>}
      {details.executor !== null && (
        <>
          <PreviewFact label={t("bindings.model")}>{details.model === null ? t("effort.executor") : cleanDisplayText(details.model)}</PreviewFact>
          <PreviewFact label={t("bindings.effort")}>{t(details.effort === null ? "effort.executor" : `effort.${details.effort}`)}</PreviewFact>
        </>
      )}
      {details.permission !== null && (
        <PreviewFact label={t("bindings.permission")}>
          {details.permission === "verification-sandbox-network" ? t("acceptance.sandbox")
            : details.permission === "verification-sandbox" ? t("acceptance.sandbox_legacy") : cleanDisplayText(details.permission)}
        </PreviewFact>
      )}
      {details.lean !== null && <PreviewFact label={t("bindings.lean")}>{t(`bindings.lean_state.${details.lean}`)}</PreviewFact>}
    </dl>
  );
}

function ReviewAction({ job, kind, sourceId, close }: {
  job: Job;
  kind: AcceptanceAction["action"];
  sourceId: number;
  close: () => void;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const id = useId();
  const start = kind === "start_verification";
  const [executor, setExecutor] = useState<ExecutorKind | null>(job.executor);
  const [text, setText] = useState("");
  const preview = useAcceptancePreview();
  const approval = useCommitAcceptance();
  const action = text.trim() === "" ? null : start
    ? executor === null ? null : { action: kind, job_id: job.id, executor, delivery: text.trim() } as const
    : { action: kind, job_id: job.id, reason: text.trim() } as const;
  const data = preview.data;
  const details = data?.acceptance ?? null;
  const reviewed = data?.action;
  const matching = data !== undefined && details !== null && action !== null && reviewed?.job_id === job.id &&
    details.source_job_id === sourceId && data.issue_identifier === job.issue_identifier &&
    data.job_id === job.id && data.current_state === job.state &&
    (action.action === "start_verification"
      ? reviewed.action === action.action && reviewed.executor === action.executor && reviewed.delivery === action.delivery &&
        details.executor === action.executor && details.delivery === action.delivery
      : reviewed.action === action.action && reviewed.reason === action.reason);
  const busy = preview.isPending || approval.isPending;
  const reset = () => {
    preview.reset();
    approval.reset();
  };
  const prepare = () => {
    if (action === null || busy) return;
    approval.reset();
    preview.mutate(action);
  };
  const confirm = () => {
    if (!matching || busy || approval.isError) return;
    void approval.mutateAsync(data.token).then(
      (result) => {
        close();
        if (result.result === "verification_queued" && result.job_id !== null) void navigate(`/runs/${result.job_id}`);
      },
      () => undefined,
    );
  };
  const preventCloseWhileSaving = (event: Event) => {
    if (approval.isPending) event.preventDefault();
  };
  return (
    <DialogContent
      className="max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] sm:max-w-lg"
      showCloseButton={false}
      onEscapeKeyDown={preventCloseWhileSaving}
      onInteractOutside={preventCloseWhileSaving}
    >
      <DialogHeader>
        <DialogTitle>{t(start ? "acceptance.start" : "acceptance.repair")}</DialogTitle>
        <DialogDescription>{t(start ? "acceptance.start_description" : "acceptance.repair_description")}</DialogDescription>
      </DialogHeader>
      <div className="-m-1 min-h-0 space-y-4 overflow-y-auto p-1">
        {start && (
          <>
            <div className="space-y-2">
              <p id={`${id}-executor`} className="font-medium">{t("runs.field.executor")}</p>
              <RadioGroup
                className="flex flex-wrap gap-x-6 gap-y-2"
                aria-labelledby={`${id}-executor`}
                value={executor ?? ""}
                disabled={busy}
                onValueChange={(value) => {
                  if (value !== "codex" && value !== "claude") return;
                  reset();
                  setExecutor(value);
                }}
              >
                {(["codex", "claude"] as const).map((kind) => (
                  <div className="flex items-center gap-2" key={kind}>
                    <RadioGroupItem id={`${id}-${kind}`} value={kind} />
                    <Label htmlFor={`${id}-${kind}`}>{t(`executor.${kind}`)}</Label>
                  </div>
                ))}
              </RadioGroup>
            </div>
            <p className="text-sm text-muted-foreground">{t(
              details?.permission === "verification-sandbox" ? "acceptance.sandbox_legacy_hint" : "acceptance.sandbox_hint",
            )}</p>
          </>
        )}
        <div className="space-y-2">
          <Label htmlFor={id}>{t(start ? "acceptance.delivery" : "acceptance.findings")}</Label>
          <Textarea
            id={id}
            aria-describedby={start ? `${id}-hint` : undefined}
            rows={start ? 2 : 4}
            value={text}
            disabled={busy}
            onChange={(event) => {
              reset();
              setText(event.target.value);
            }}
          />
          {start && <p id={`${id}-hint`} className="text-sm text-muted-foreground">{t("acceptance.delivery_hint")}</p>}
        </div>
        {preview.isPending && <LoadingState rows={2} />}
        {preview.isError && <ErrorState error={preview.error} headline="dispatch.preview_failed" />}
        {preview.isSuccess && (matching
          ? (
            <>
              <ActionFacts preview={data} details={details} />
              {details.context != null && (
                <dl className="space-y-3">
                  <PreviewContext context={details.context} scope={details.scope ?? details.context.scope} />
                </dl>
              )}
              {details.context === null && <p className="text-sm text-muted-foreground">{t("dispatch.empty_context")}</p>}
            </>
          )
          : <p role="alert" className="text-sm">{t("acceptance.preview_mismatch")}</p>)}
        {approval.isError && <ErrorState error={approval.error} headline="dispatch.approval_failed" />}
      </div>
      <DialogFooter className="flex-wrap">
        <DialogClose asChild>
          <Button variant="outline" disabled={approval.isPending}>{t("common.cancel")}</Button>
        </DialogClose>
        <Button variant="outline" disabled={action === null || busy} onClick={prepare}>
          {t(data !== undefined || approval.isError ? "dispatch.reload_preview" : "acceptance.action_preview")}
        </Button>
        <Button disabled={!matching || busy || approval.isError} onClick={confirm}>{t("acceptance.action_confirm")}</Button>
      </DialogFooter>
    </DialogContent>
  );
}

function AcceptanceActionButton({ job, kind, sourceId = job.id }: { job: Job; kind: AcceptanceAction["action"]; sourceId?: number }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button size="sm" variant="outline">{t(kind === "start_verification" ? "acceptance.start" : "acceptance.repair")}</Button>
      </DialogTrigger>
      {open && <ReviewAction job={job} kind={kind} sourceId={sourceId} close={() => setOpen(false)} />}
    </Dialog>
  );
}

function LinkedAcceptance({ job, links, archiveConfigured }: { job: Job; links: AcceptanceLinks; archiveConfigured: boolean }) {
  const { t } = useTranslation();
  const awaitsAcceptance = job.state === "local_complete" || job.state === "awaiting_acceptance";
  const verificationId = links.verification_job_id;
  const archiveNeeded = awaitsAcceptance && archiveConfigured && verificationId !== null && links.verification_status === "passed";
  const verification = useHandoff(verificationId ?? 0, archiveNeeded);
  const linkStyle = "text-primary underline-offset-4 hover:underline";
  return (
    <>
      <dl className="grid gap-3 sm:grid-cols-2">
        {links.development_job_id !== job.id && (
          <PreviewFact label={t("continuation.source_run")}>
            <Link className={linkStyle} to={`/runs/${links.development_job_id}`}>
              {t("acceptance.development_run", { id: links.development_job_id })}
            </Link>
          </PreviewFact>
        )}
        {links.verification_status !== null && job.mode !== "verify" && (
          <PreviewFact label={t("acceptance.status")}>
            {t(`acceptance.verification_status.${links.verification_status}`)}
          </PreviewFact>
        )}
        {verificationId !== null && verificationId !== job.id && (
          <PreviewFact label={t("acceptance.title")}>
            <Link className={linkStyle} to={`/runs/${verificationId}`}>
              {t("acceptance.verification_run", { id: verificationId })}
            </Link>
          </PreviewFact>
        )}
        {links.repair_of_job_id !== null && (
          <PreviewFact label={t("acceptance.repair_source")}>
            <Link className={linkStyle} to={`/runs/${links.repair_of_job_id}`}>
              {t("runs.run_label", { id: links.repair_of_job_id })}
            </Link>
          </PreviewFact>
        )}
        {links.repair_job_id != null && links.repair_job_id !== job.id && (
          <PreviewFact label={t("acceptance.repair")}>
            <Link className={linkStyle} to={`/runs/${links.repair_job_id}`}>
              {t("acceptance.repair_run", { id: links.repair_job_id })}
            </Link>
          </PreviewFact>
        )}
        {links.delivery !== null && <PreviewFact label={t("acceptance.delivery")}>{cleanDisplayLines(links.delivery)}</PreviewFact>}
        {links.repair_reason !== null && <PreviewFact label={t("acceptance.findings")}>{cleanDisplayLines(links.repair_reason)}</PreviewFact>}
      </dl>
      {links.verification_evidence != null && links.verification_evidence.length > 0 && (
        <dl>
          <PreviewFact label={t("acceptance.evidence")}>
            <ul className="list-disc space-y-1 ps-5">
              {links.verification_evidence.map((entry, index) => <li key={index}>{cleanDisplayLines(entry)}</li>)}
            </ul>
          </PreviewFact>
        </dl>
      )}
      {awaitsAcceptance && !links.returned_for_repair &&
        (verificationId !== null || links.repair_of_job_id !== null) && links.verification_status !== "passed" && (
          <p className="text-sm text-muted-foreground">{t("acceptance.completion_blocked")}</p>
        )}
      {archiveNeeded && verification.isSuccess && verification.data.archive?.receipt == null && (
        <p className="text-sm text-muted-foreground">{t("acceptance.verification_archive_needed")}</p>
      )}
      {archiveNeeded && verification.isError && (
        <ErrorState error={verification.error} headline="acceptance.read_failed" onRetry={() => void verification.refetch()} />
      )}
      {links.returned_for_repair && (
        <div className="space-y-2">
          <p className="font-medium">{t("acceptance.returned")}</p>
          {links.repair_job_id == null && (
            <>
              <p className="text-sm text-muted-foreground">{t("acceptance.issue_updated")}</p>
              <DispatchReviewButton issueIdentifier={job.issue_identifier} issueUuid={job.issue_uuid} />
            </>
          )}
        </div>
      )}
    </>
  );
}

export function AcceptanceCard({ job }: { job: Job }) {
  const { t } = useTranslation();
  const id = useId();
  const handoff = useHandoff(job.id);
  const jobs = useJobs();
  if (job.mode === "plan") return null;
  const links = handoff.data?.acceptance ?? null;
  const development = job.mode !== "verify" && job.started_at !== null && job.finished_at !== null &&
    (job.state === "local_complete" || job.state === "awaiting_acceptance");
  if (!development && links === null && job.mode !== "verify") return null;
  const latestDevelopment = jobs.data?.find((other) => other.issue_uuid === job.issue_uuid && other.mode !== "verify");
  const canStart = development && latestDevelopment?.id === job.id && handoff.isSuccess && handoff.data.claimed_by === null &&
    !links?.returned_for_repair && (links?.verification_status == null || links.verification_status === "blocked");
  const canRepair = job.mode === "verify" && links?.verification_status === "failed" && !links.returned_for_repair &&
    latestDevelopment?.id === links.development_job_id;
  return (
    <section aria-labelledby={id}>
      <Card size="sm">
        <CardContent className="space-y-3">
          <h2 id={id} className="font-medium">{t("acceptance.title")}</h2>
          {job.mode === "verify" && <p className="text-sm text-muted-foreground">{t("acceptance.temporary_copy")}</p>}
          {handoff.isError && <ErrorState error={handoff.error} headline="acceptance.read_failed" onRetry={() => void handoff.refetch()} />}
          {handoff.isPending && <LoadingState rows={1} />}
          {links && <LinkedAcceptance job={job} links={links} archiveConfigured={handoff.data?.archive != null} />}
          {development && handoff.isSuccess && links?.verification_status == null && !links?.returned_for_repair && (
            <p className="text-sm text-muted-foreground">{t("acceptance.none")}</p>
          )}
          {(canStart || canRepair) && (
            <div className="flex flex-wrap gap-2">
              {canStart && <AcceptanceActionButton job={job} kind="start_verification" />}
              {canRepair && <AcceptanceActionButton job={job} kind="return_for_repair" sourceId={links.development_job_id} />}
            </div>
          )}
        </CardContent>
      </Card>
    </section>
  );
}

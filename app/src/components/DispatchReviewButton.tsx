import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,
} from "@/components/ui/dialog";
import { useApproveDispatch, useDispatchPreview, useOpenDevelopmentIssue } from "@/hooks/queries";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import { cleanDisplayText } from "@/lib/clean";
import type { DispatchPreview, IssueContext } from "@/lib/workflow-types";

export function PreviewFact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="wrap-anywhere whitespace-pre-wrap">{children}</dd>
    </div>
  );
}

function PreviewList({ values }: { values: string[] }) {
  if (values.length === 0) return "—";
  return (
    <ul className="list-disc ps-4">
      {values.map((value, index) => <li key={index}>{value}</li>)}
    </ul>
  );
}

/** The exact brief returned by the preview, rendered as plain text. */
export function PreviewContext({ context, scope = context.scope }: { context: IssueContext; scope?: string }) {
  const { t } = useTranslation();
  const conversation = context.conversation_history;
  return (
    <>
      <PreviewFact label={t("dispatch.summary")}>{context.request}</PreviewFact>
      {conversation && <PreviewFact label={t("dispatch.conversation_history")}>{conversation}</PreviewFact>}
      <PreviewFact label={t("handoff.scope")}>{scope}</PreviewFact>
      <PreviewFact label={t("handoff.acceptance")}><PreviewList values={context.acceptance} /></PreviewFact>
      <PreviewFact label={t("dispatch.wiki_refs")}><PreviewList values={context.source_refs} /></PreviewFact>
    </>
  );
}

export function PreviewDetails({ preview }: { preview: DispatchPreview }) {
  const { t } = useTranslation();
  const { context } = preview;
  // The core validates context before it becomes a preview. Render it as plain text so
  // indentation and line breaks match the exact approved payload.
  const wikiFolder = cleanDisplayText(preview.wiki_root ?? "").trim();
  return (
    <div className="flex flex-col gap-4">
      <dl className="grid grid-cols-2 gap-3">
        <PreviewFact label={t("runs.field.issue")}>
          {cleanDisplayText(preview.issue_identifier)} · {cleanDisplayText(preview.title)}
        </PreviewFact>
        <PreviewFact label={t("runs.field.mode")}>{t(preview.mode === "plan" ? "runs.mode.plan" : "dispatch.implement")}</PreviewFact>
        <PreviewFact label={t("runs.field.executor")}>{t(`executor.${preview.executor}`)}</PreviewFact>
        <PreviewFact label={t("runs.field.directory")}>{cleanDisplayText(preview.directory)}</PreviewFact>
        <PreviewFact label={t("bindings.model")}>{preview.model === null ? t("effort.executor") : cleanDisplayText(preview.model)}</PreviewFact>
        <PreviewFact label={t("bindings.effort")}>{t(preview.effort === null ? "effort.executor" : `effort.${preview.effort}`)}</PreviewFact>
      </dl>
      {context === null && <p className="text-muted-foreground">{t("dispatch.empty_context")}</p>}
      {(context !== null || wikiFolder !== "") && (
        <dl className="space-y-3">
          {context !== null && <PreviewContext context={context} />}
          {wikiFolder !== "" && <PreviewFact label={t("handoff.archive.root")}>{wikiFolder}</PreviewFact>}
        </dl>
      )}
    </div>
  );
}

interface ReviewTarget {
  issueIdentifier: string;
  issueUuid: string;
  developmentJobId?: number;
}

function DevelopmentSetup({ jobId, reload }: { jobId: number; reload: () => void }) {
  const { t } = useTranslation();
  const { todo, label } = useTriggerNames();
  const openIssue = useOpenDevelopmentIssue(jobId);
  return (
    <div className="space-y-2">
      <p className="text-muted-foreground">{t("dispatch.prepare_implementation", { todo, label })}</p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" disabled={openIssue.isPending} onClick={() => openIssue.mutate()}>
          {t("actions.open_in_linear")}
        </Button>
        <Button size="sm" variant="outline" onClick={reload}>{t("dispatch.reload_preview")}</Button>
      </div>
    </div>
  );
}

function ReviewDialog({ issueIdentifier, issueUuid, developmentJobId, close }: ReviewTarget & { close: () => void }) {
  const { t } = useTranslation();
  const preview = useDispatchPreview(issueIdentifier);
  const approval = useApproveDispatch(issueIdentifier);
  const matching = preview.isSuccess && preview.data.issue_uuid === issueUuid ? preview.data : null;
  const canDispatch = matching !== null && (developmentJobId === undefined || matching.mode === "implement");
  const ready = canDispatch && !preview.isFetching && !approval.isPending && !approval.isError;
  const needsSetup = developmentJobId !== undefined && !canDispatch && !preview.isFetching;
  const approve = () => {
    if (!ready || matching === null) return;
    approval.mutate(matching.token, { onSuccess: close });
  };
  const reload = () => {
    approval.reset();
    void preview.refetch();
  };
  const keepOpenWhileSaving = (event: Event) => {
    if (approval.isPending) event.preventDefault();
  };
  return (
    <DialogContent
      className="max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] sm:max-w-lg"
      showCloseButton={false}
      onEscapeKeyDown={keepOpenWhileSaving}
      onInteractOutside={keepOpenWhileSaving}
    >
      <DialogHeader>
        <DialogTitle>{t("dispatch.title")}</DialogTitle>
        <DialogDescription>{t("dispatch.description")}</DialogDescription>
      </DialogHeader>
      <div className="-m-1 min-h-0 space-y-3 overflow-y-auto p-1">
        {preview.isFetching ? (
          <LoadingState rows={3} />
        ) : preview.isError ? (
          <ErrorState error={preview.error} headline="dispatch.preview_failed" onRetry={needsSetup ? undefined : reload} />
        ) : matching !== null ? (
          <PreviewDetails preview={matching} />
        ) : preview.isSuccess && (
          <>
            <p role="alert">{t("dispatch.issue_mismatch")}</p>
            {!needsSetup && <Button size="sm" variant="outline" onClick={reload}>{t("dispatch.reload_preview")}</Button>}
          </>
        )}
        {needsSetup && <DevelopmentSetup jobId={developmentJobId} reload={reload} />}
        {approval.isError && (
          <>
            <ErrorState error={approval.error} headline="dispatch.approval_failed" />
            <Button size="sm" variant="outline" onClick={reload}>{t("dispatch.reload_preview")}</Button>
          </>
        )}
      </div>
      <DialogFooter>
        <DialogClose asChild>
          <Button variant="outline" disabled={approval.isPending}>{t("common.cancel")}</Button>
        </DialogClose>
        <Button disabled={!ready} onClick={approve}>
          {t("dispatch.confirm")}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}

/** Loading the preview never approves it. Closing discards the copy held by the panel. */
export function DispatchReviewButton(target: ReviewTarget) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline" size="sm">{t(target.developmentJobId === undefined ? "dispatch.preview" : "dispatch.implement")}</Button>
      </DialogTrigger>
      {open && <ReviewDialog {...target} close={() => setOpen(false)} />}
    </Dialog>
  );
}

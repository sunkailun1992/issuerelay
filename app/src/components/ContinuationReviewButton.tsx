import { Play } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { PreviewDetails, PreviewFact } from "@/components/DispatchReviewButton";
import { InformationRequested } from "@/components/HandoffCard";
import { ErrorState, LoadingState } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Textarea } from "@/components/ui/textarea";
import { useApproveContinuation, useContinuationContext, useContinuationPreview, useHandoff, useSupplementContinuationContext } from "@/hooks/queries";
import { cleanDisplayLines } from "@/lib/clean";
import type { ExecutorKind, Job } from "@/lib/types";

const EXECUTORS = ["codex", "claude"] as const;

function RequestedInformation({ id }: { id: number }) {
  const handoff = useHandoff(id);
  if (handoff.isError && !handoff.isFetching) {
    return <ErrorState error={handoff.error} headline="handoff.read_failed" onRetry={() => void handoff.refetch()} />;
  }
  if (handoff.data === undefined) return <LoadingState rows={1} />;
  return <InformationRequested blockers={handoff.data.memory?.checkpoint?.blockers ?? []} />;
}

function ReviewDialog({ source, close }: { source: Job; close: () => void }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const id = useId();
  const needsInfo = source.state === "needs_info";
  const [executor, setExecutor] = useState<ExecutorKind | null>(null);
  const [supplement, setSupplement] = useState("");
  const context = useContinuationContext(source.id, needsInfo);
  const supplementation = useSupplementContinuationContext(source.id);
  const preview = useContinuationPreview(source.id, executor);
  const approval = useApproveContinuation(source.id);
  const data = preview.data;
  const matching = data != null && data.issue_uuid === source.issue_uuid && data.source_job_id === source.id &&
    data.source_executor === source.executor && data.executor === executor &&
    (!needsInfo || (context.isSuccess && !context.isFetching && (data.context?.revision ?? 0) === (context.data?.revision ?? 0))) ? data : null;
  const saving = approval.isPending || supplementation.isPending;
  const hasSupplement = supplement.trim() !== "";
  const ready = matching !== null && !preview.isFetching && !preview.isError && !saving &&
    !approval.isError && !supplementation.isError && !hasSupplement;
  const approve = () => {
    if (!ready || matching === null) return;
    void approval.mutateAsync(matching.token).then(
      (jobId) => {
        close();
        void navigate(`/runs/${jobId}`);
      },
      () => undefined,
    );
  };
  const reload = () => {
    approval.reset();
    supplementation.reset();
    if (needsInfo) void context.refetch();
    void preview.refetch();
  };
  const saveSupplement = () => {
    if (!context.isSuccess || context.isFetching || !hasSupplement || saving) return;
    void supplementation.mutateAsync({
      expectedRevision: context.data?.revision ?? 0,
      supplement,
    }).then(() => {
      setSupplement("");
      approval.reset();
      if (executor !== null) void preview.refetch();
    }, () => undefined);
  };
  const keepOpenWhileSaving = (event: Event) => {
    if (saving) event.preventDefault();
  };
  return (
    <DialogContent
      className="max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] sm:max-w-lg"
      showCloseButton={false}
      onEscapeKeyDown={keepOpenWhileSaving}
      onInteractOutside={keepOpenWhileSaving}
    >
      <DialogHeader>
        <DialogTitle>{t("continuation.action")}</DialogTitle>
        <DialogDescription>{t("continuation.description")}</DialogDescription>
      </DialogHeader>
      <div className="-m-1 min-h-0 space-y-4 overflow-y-auto p-1">
        {needsInfo && (
          <div className="space-y-2">
            <RequestedInformation id={source.id} />
            <Label htmlFor={`${id}-supplement`}>{t("continuation.supplement_label")}</Label>
            <p id={`${id}-supplement-hint`} className="text-sm text-muted-foreground">{t("continuation.supplement_hint")}</p>
            {context.isPending ? <LoadingState rows={1} /> : context.isError ? (
              <ErrorState error={context.error} headline="continuation.supplement_failed" onRetry={() => void context.refetch()} />
            ) : (
              <>
                <Textarea
                  id={`${id}-supplement`}
                  aria-describedby={`${id}-supplement-hint`}
                  value={supplement}
                  disabled={saving}
                  rows={4}
                  onChange={(event) => setSupplement(event.target.value)}
                />
                <Button size="sm" variant="outline" disabled={!hasSupplement || saving || context.isFetching} onClick={saveSupplement}>
                  {t("continuation.supplement_preview")}
                </Button>
              </>
            )}
            {supplementation.isError && (
              <ErrorState error={supplementation.error} headline="continuation.supplement_failed" onRetry={reload} />
            )}
          </div>
        )}
        <div className="space-y-2">
          <p id={`${id}-label`} className="font-medium">{t("handoff.executor")}</p>
          <RadioGroup
            className="flex flex-wrap gap-x-6 gap-y-2"
            aria-labelledby={`${id}-label`}
            value={executor ?? ""}
            disabled={saving}
            onValueChange={(value) => {
              if (value !== "codex" && value !== "claude") return;
              approval.reset();
              setExecutor(value);
            }}
          >
            {EXECUTORS.map((kind) => (
              <div className="flex items-center gap-2" key={kind}>
                <RadioGroupItem id={`${id}-${kind}`} value={kind} />
                <Label htmlFor={`${id}-${kind}`}>{t(`executor.${kind}`)}</Label>
              </div>
            ))}
          </RadioGroup>
        </div>
        {preview.isFetching ? (
          <LoadingState rows={3} />
        ) : preview.isError ? (
          <ErrorState error={preview.error} headline="dispatch.preview_failed" onRetry={reload} />
        ) : matching !== null ? (
          <>
            <dl className="grid grid-cols-2 gap-3">
              <PreviewFact label={t("continuation.source_run")}>
                {t("continuation.source_value", { id: source.id, executor: t(`executor.${matching.source_executor}`) })}
              </PreviewFact>
              <PreviewFact label={t("continuation.strategy")}>{t(`continuation.${matching.strategy}`)}</PreviewFact>
              <PreviewFact label={t("bindings.permission")}>{t(`permission.${matching.permission}`)}</PreviewFact>
            </dl>
            <PreviewDetails preview={matching} />
            {matching.claude_ultracode === true && <Badge variant="outline">{t("settings.claude_ultracode")}</Badge>}
            <dl>
              <PreviewFact label={t("continuation.evidence")}>{cleanDisplayLines(matching.evidence)}</PreviewFact>
            </dl>
          </>
        ) : preview.isSuccess && (
          <>
            <p role="alert">{t("continuation.preview_mismatch")}</p>
            <Button size="sm" variant="outline" onClick={reload}>{t("dispatch.reload_preview")}</Button>
          </>
        )}
        {approval.isError && (
          <>
            <ErrorState error={approval.error} headline="dispatch.approval_failed" />
            <Button size="sm" variant="outline" onClick={reload}>{t("dispatch.reload_preview")}</Button>
          </>
        )}
      </div>
      <DialogFooter>
        <DialogClose asChild>
          <Button variant="outline" disabled={saving}>{t("common.cancel")}</Button>
        </DialogClose>
        <Button disabled={!ready} onClick={approve}>{t("continuation.confirm")}</Button>
      </DialogFooter>
    </DialogContent>
  );
}

/** Preview and confirm one explicit choice, using the earlier run as the source. */
export function ContinuationReviewButton({ source, size }: { source: Job; size: "default" | "sm" }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline" size={size}>
          {size === "default" && <Play aria-hidden />}
          {t("continuation.action")}
        </Button>
      </DialogTrigger>
      {open && <ReviewDialog source={source} close={() => setOpen(false)} />}
    </Dialog>
  );
}

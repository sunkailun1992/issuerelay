import { Play } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { PreviewDetails, PreviewFact } from "@/components/DispatchReviewButton";
import { ErrorState, LoadingState } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useApproveContinuation, useContinuationPreview } from "@/hooks/queries";
import { cleanDisplayLines } from "@/lib/clean";
import type { ExecutorKind, Job } from "@/lib/types";

const EXECUTORS = ["codex", "claude"] as const;

function ReviewDialog({ source, close }: { source: Job; close: () => void }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const id = useId();
  const [executor, setExecutor] = useState<ExecutorKind | null>(null);
  const preview = useContinuationPreview(source.id, executor);
  const approval = useApproveContinuation(source.id);
  const data = preview.data;
  const matching = data != null && data.issue_uuid === source.issue_uuid && data.source_job_id === source.id &&
    data.source_executor === source.executor && data.executor === executor ? data : null;
  const ready = matching !== null && !preview.isFetching && !preview.isError && !approval.isPending && !approval.isError;
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
        <DialogTitle>{t("continuation.action")}</DialogTitle>
        <DialogDescription>{t("continuation.description")}</DialogDescription>
      </DialogHeader>
      <div className="-m-1 min-h-0 space-y-4 overflow-y-auto p-1">
        <div className="space-y-2">
          <p id={`${id}-label`} className="font-medium">{t("handoff.executor")}</p>
          <RadioGroup
            className="flex flex-wrap gap-x-6 gap-y-2"
            aria-labelledby={`${id}-label`}
            value={executor ?? ""}
            disabled={approval.isPending}
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
          <Button variant="outline" disabled={approval.isPending}>{t("common.cancel")}</Button>
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

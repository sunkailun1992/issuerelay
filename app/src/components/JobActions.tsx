import { Check, ExternalLink, RotateCcw, Square } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ContinuationReviewButton } from "@/components/ContinuationReviewButton";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { useCancelJob, useJobs, useMarkDone, useRetryJob } from "@/hooks/queries";
import { useOpenLinear } from "@/hooks/useOpenLinear";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import { isActive, isInterruptedRun, isNewestRun, pendingContinuationSourceId } from "@/lib/runs";
import { sameName } from "@/lib/settings-form";
import type { IssueView, Job, JobState } from "@/lib/types";
import { cn } from "@/lib/utils";

/** The runs that can be run again: they ended without the result the issue needed. */
const RETRYABLE: readonly JobState[] = ["failed", "canceled", "needs_info"];

/** The size of a text button: the compact form sits in a table row or a list. */
type Size = "default" | "sm";

function StopButton({ job, size }: { job: Job; size: Size }) {
  const { t } = useTranslation();
  const cancel = useCancelJob();
  return (
    <Button variant="outline" size={size} disabled={cancel.isPending} onClick={() => cancel.mutate(job.id)}>
      <Square aria-hidden className="fill-current" />
      {t("actions.stop")}
    </Button>
  );
}

function RetryButton({ job, size }: { job: Job; size: Size }) {
  const { t } = useTranslation();
  const jobs = useJobs();
  const retry = useRetryJob();
  // The engine only runs the newest run of an issue again. Until the list of runs has been read
  // (or when it cannot be) the button is there as usual; the engine refuses an older run.
  if (jobs.data !== undefined && !isNewestRun(job, jobs.data)) return null;
  return (
    <Button variant="outline" size={size} disabled={retry.isPending} onClick={() => retry.mutate(job.id)}>
      <RotateCcw aria-hidden />
      {t("actions.run_again")}
    </Button>
  );
}

function ContinueButton({ job, source, size }: { job: Job; source: Job; size: Size }) {
  const jobs = useJobs();
  const latest = jobs.data?.find((other) => other.issue_uuid === job.issue_uuid);
  if (source.state === "needs_info" && latest === undefined) return null;
  if (latest && latest.id > job.id && pendingContinuationSourceId(latest) !== source.id) return null;
  return <ContinuationReviewButton source={source} size={size} />;
}

/**
 * Marking done is the one change IssueRelay makes in Linear, so it asks first and says so. The
 * status it sets is the configured one (the settings' "done" name).
 */
function MarkDoneButton({ job, size }: { job: Job; size: Size }) {
  const { t } = useTranslation();
  const markDone = useMarkDone();
  const { done } = useTriggerNames();
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button size={size} disabled={markDone.isPending}>
          <Check aria-hidden />
          {t("actions.mark_done")}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("actions.mark_done")}</AlertDialogTitle>
          <AlertDialogDescription>{t("actions.mark_done_confirm", { identifier: job.issue_identifier, done })}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction onClick={() => markDone.mutate(job.id)}>{t("actions.confirm")}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** Opens the issue in the browser (`useOpenLinear`: the backend decides what may be opened). */
function OpenInLinearButton({ url, compact }: { url: string; compact: boolean }) {
  const { t } = useTranslation();
  const open = useOpenLinear();
  const label = t("actions.open_in_linear");
  // Compact: an icon that keeps its name for screen readers and as a tooltip.
  return compact ? (
    <Button variant="ghost" size="icon-sm" aria-label={label} title={label} onClick={() => open(url)}>
      <ExternalLink aria-hidden />
    </Button>
  ) : (
    <Button variant="outline" onClick={() => open(url)}>
      <ExternalLink aria-hidden />
      {label}
    </Button>
  );
}

/**
 * What can be done with a run, shared by the overview, the issue list and the run's page: only
 * the actions its state allows, and "Open in Linear" when the issue's link is given. A compact row
 * reserves one text button for its primary action. On the run's page, Continue and Run again
 * remain separate actions.
 */
export function JobActions({
  job,
  url,
  compact = false,
  continuationSource = isInterruptedRun(job) ? job : null,
  localAcceptance,
}: {
  job?: Job | null;
  url?: string;
  compact?: boolean;
  continuationSource?: Job | null;
  localAcceptance?: { view: IssueView; inTesting: string };
}) {
  const size: Size = compact ? "sm" : "default";
  const continuable = job != null && continuationSource !== null && (!compact || job.state !== "queued");
  const showOtherActions = !compact || !continuable;
  const stoppable = showOtherActions && job != null && isActive(job);
  const retryable = showOtherActions && job != null && RETRYABLE.includes(job.state);
  // The shared issue list supplies the latest run and last Linear status read. The engine checks again on confirmation.
  const markable = job != null && (job.state === "awaiting_acceptance" || (
    job.state === "local_complete" && localAcceptance !== undefined &&
    localAcceptance.view.issue.uuid === job.issue_uuid && localAcceptance.view.job?.id === job.id &&
    sameName(localAcceptance.view.issue.state, localAcceptance.inTesting)
  ));
  if (!stoppable && !retryable && !continuable && !markable && !url) return null;
  return (
    <div className={cn("flex items-center gap-2", compact ? "flex-nowrap" : "flex-wrap")}>
      {stoppable && <StopButton job={job} size={size} />}
      {continuable && <ContinueButton job={job} source={continuationSource} size={size} />}
      {retryable && <RetryButton job={job} size={size} />}
      {markable && <MarkDoneButton job={job} size={size} />}
      {url && <OpenInLinearButton url={url} compact={compact} />}
    </div>
  );
}

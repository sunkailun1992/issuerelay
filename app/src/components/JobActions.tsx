import { Check, ExternalLink, RotateCcw, Square } from "lucide-react";
import { useTranslation } from "react-i18next";
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
import { isActive, isNewestRun } from "@/lib/runs";
import type { Job, JobState } from "@/lib/types";
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
 * the actions its state allows (stop while it is in hand; run again after a failure, a stop or a
 * question, if it is its issue's newest run; mark done once it awaits acceptance), and "Open in
 * Linear" when the issue's link is given. Without a run only the link is offered. `compact` is
 * for a row of a table or a list: smaller buttons that keep to one line, and a link that is an
 * icon. A table that cannot spare even that gives no `url` and makes the identifier the link
 * (`IssueLink`), as the issue list does.
 */
export function JobActions({ job, url, compact = false }: { job?: Job | null; url?: string; compact?: boolean }) {
  const size: Size = compact ? "sm" : "default";
  const stoppable = job != null && isActive(job);
  const retryable = job != null && RETRYABLE.includes(job.state);
  const markable = job != null && job.state === "awaiting_acceptance";
  if (!stoppable && !retryable && !markable && !url) return null;
  return (
    <div className={cn("flex items-center gap-2", compact ? "flex-nowrap" : "flex-wrap")}>
      {stoppable && <StopButton job={job} size={size} />}
      {retryable && <RetryButton job={job} size={size} />}
      {markable && <MarkDoneButton job={job} size={size} />}
      {url && <OpenInLinearButton url={url} compact={compact} />}
    </div>
  );
}

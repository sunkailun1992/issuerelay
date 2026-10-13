import { CircleAlert, Copy, Info, SearchX } from "lucide-react";
import { type ReactNode, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useParams } from "react-router";
import { JobActions } from "@/components/JobActions";
import { PageHeader } from "@/components/PageHeader";
import { RawLog } from "@/components/RawLog";
import { RunDuration } from "@/components/RunDuration";
import { RunTimeline } from "@/components/RunTimeline";
import { StatusBadge } from "@/components/StatusBadge";
import { DetailLine, EmptyState, ErrorState, LoadingState } from "@/components/states";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useIssues, useJob, useJobLog } from "@/hooks/queries";
import { useCopyText } from "@/hooks/useCopyText";
import { useLang } from "@/hooks/useLang";
import { cleanDisplayLines, cleanDisplayText } from "@/lib/clean";
import { formatDateTime } from "@/lib/format";
import { isActive } from "@/lib/runs";
import { parseTimeline } from "@/lib/timeline";
import type { Job } from "@/lib/types";
import { cn } from "@/lib/utils";

/** What stands where a field has no value. */
const NONE = "—";

/** One fact about the run: its name above, its value below. */
function Field({ label, wide = false, children }: { label: string; wide?: boolean; children: ReactNode }) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-0.5", wide && "col-span-full")}>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="min-w-0 wrap-anywhere">{children}</dd>
    </div>
  );
}

/** The folder the run works in, in monospace, with a button to copy it (the path as it is shown: cleaned). */
function Folder({ directory }: { directory: string }) {
  const { t } = useTranslation();
  const copy = useCopyText();
  const shown = cleanDisplayText(directory).trim();
  return (
    <span className="flex items-center gap-2">
      <code className="min-w-0 font-mono text-xs wrap-anywhere">{shown}</code>
      <Button variant="ghost" size="icon-xs" aria-label={t("common.copy")} onClick={() => copy(shown)}>
        <Copy aria-hidden />
      </Button>
    </span>
  );
}

/** The facts about the run (design 9.2). The summary is the AI's own words, cleaned. */
function InfoCard({ job }: { job: Job }) {
  const { t } = useTranslation();
  const lang = useLang();
  const time = (iso: string | null) => (iso === null ? NONE : formatDateTime(iso, lang));
  const summary = job.summary === null ? "" : cleanDisplayLines(job.summary).trim();
  return (
    <Card size="sm">
      <CardContent>
        {/* As many columns as the card has room for, down to the 900 px window's three. */}
        <dl className="grid grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] gap-x-6 gap-y-3">
          <Field label={t("runs.field.state")}>
            <StatusBadge state={job.state} />
          </Field>
          <Field label={t("runs.field.executor")}>{job.executor === null ? NONE : t(`executor.${job.executor}`)}</Field>
          <Field label={t("runs.field.created")}>{time(job.created_at)}</Field>
          <Field label={t("runs.field.started")}>{time(job.started_at)}</Field>
          <Field label={t("runs.field.finished")}>{time(job.finished_at)}</Field>
          <Field label={t("runs.field.duration")}>
            <RunDuration job={job} />
          </Field>
          <Field label={t("runs.field.directory")} wide>
            {job.directory === null ? NONE : <Folder directory={job.directory} />}
          </Field>
          <Field label={t("runs.field.summary")} wide>
            <span className="whitespace-pre-wrap">{summary === "" ? NONE : summary}</span>
          </Field>
        </dl>
      </CardContent>
    </Card>
  );
}

/** The states a reason has a headline of its own for; any other state's is a note from IssueRelay. */
function reasonKind(job: Job): "failed" | "canceled" | "waiting_binding" | "other" {
  return job.state === "failed" || job.state === "canceled" || job.state === "waiting_binding" ? job.state : "other";
}

/**
 * Why the run is where it is. The headline is from the state, in the interface's words; the
 * engine's own sentence is the detail line under it, never alone. A failure is an alert, which a
 * screen reader announces at once; the rest are notes.
 */
function Reason({ job }: { job: Job }) {
  const { t } = useTranslation();
  if (job.reason === null || cleanDisplayLines(job.reason).trim() === "") return null;
  const kind = reasonKind(job);
  const failed = kind === "failed";
  const Icon = failed ? CircleAlert : Info;
  return (
    <Alert variant={failed ? "destructive" : "default"} role={failed ? "alert" : "status"}>
      <Icon aria-hidden />
      <AlertTitle>{t(`runs.reason_title.${kind}`)}</AlertTitle>
      <AlertDescription>
        <DetailLine text={job.reason} />
      </AlertDescription>
    </Alert>
  );
}

/** What stands in a tab when the log has nothing to show: whether the log is gone or just not written yet. */
function NothingToShow({ job, noLines }: { job: Job; noLines: boolean }) {
  const { t } = useTranslation();
  // A run that ended and started had a log (a run that was stopped in the queue never did): when it
  // reads as empty, the cleanup removed it.
  const removed = noLines && !isActive(job) && job.started_at !== null;
  return <p className="rounded-lg border border-dashed p-6 text-center text-muted-foreground">{t(removed ? "runs.log_removed" : "runs.no_output")}</p>;
}

/** Both tabs read the same log: its placeholders while it is read, its error, and the empty cases. */
function LogTabs({ job }: { job: Job }) {
  const { t } = useTranslation();
  const live = job.state === "running";
  const log = useJobLog(job.id, { live });
  const lines = log.data;
  const entries = useMemo(() => (lines === undefined ? [] : parseTimeline(lines, job.executor)), [lines, job.executor]);

  /** The tab's content: what the log is like now, or `show` once it has something. */
  const panel = (show: (lines: string[]) => ReactNode) => {
    if (log.isError && !log.isFetching) {
      return <ErrorState error={log.error} headline="runs.log_failed" onRetry={() => void log.refetch()} />;
    }
    if (lines === undefined) return <LoadingState rows={3} />;
    return show(lines);
  };

  return (
    <Tabs defaultValue="timeline" className="gap-4">
      <TabsList>
        <TabsTrigger value="timeline">{t("runs.timeline")}</TabsTrigger>
        <TabsTrigger value="raw">{t("runs.raw_log")}</TabsTrigger>
      </TabsList>
      <TabsContent value="timeline">
        {panel((all) => (entries.length === 0 ? <NothingToShow job={job} noLines={all.length === 0} /> : <RunTimeline entries={entries} />))}
      </TabsContent>
      <TabsContent value="raw">
        {panel((all) => (all.length === 0 ? <NothingToShow job={job} noLines /> : <RawLog lines={all} live={live} />))}
      </TabsContent>
    </Tabs>
  );
}

/** The page of a run that exists: header, why, facts, and the log in two tabs. */
function RunView({ job }: { job: Job }) {
  const issues = useIssues();
  // `Job` has no link; the issue's own is in the issue list when that has been read and has this issue.
  const url = issues.data?.find((view) => view.issue.uuid === job.issue_uuid)?.issue.url;
  return (
    <>
      <PageHeader title={`${job.issue_identifier} ${job.title}`} actions={<JobActions job={job} url={url} />} />
      <div className="flex flex-col gap-6">
        <Reason job={job} />
        <InfoCard job={job} />
        <LogTabs job={job} />
      </div>
    </>
  );
}

function NotFound() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <>
      <PageHeader title={t("nav.runs")} />
      <EmptyState
        icon={SearchX}
        title={t("runs.not_found")}
        description={t("runs.not_found_description")}
        action={
          <Button variant="outline" size="sm" onClick={() => navigate("/runs")}>
            {t("runs.back_to_runs")}
          </Button>
        }
      />
    </>
  );
}

/** Reads the run; the page of a run that is there, or why there is none. */
function RunPage({ id }: { id: number }) {
  const { t } = useTranslation();
  const job = useJob(id);
  if (job.isError && !job.isFetching) {
    return (
      <>
        <PageHeader title={t("nav.runs")} />
        <ErrorState error={job.error} headline="runs.detail_failed" onRetry={() => void job.refetch()} />
      </>
    );
  }
  if (job.data === undefined) {
    return (
      <>
        <PageHeader title={t("nav.runs")} />
        <LoadingState rows={4} />
      </>
    );
  }
  return job.data === null ? <NotFound /> : <RunView job={job.data} />;
}

/** An address's run number: a whole number above zero, or `null`. */
function runId(text: string | undefined): number | null {
  if (text === undefined || !/^[1-9]\d*$/.test(text)) return null;
  const id = Number(text);
  return Number.isSafeInteger(id) ? id : null;
}

/**
 * One run (design 9.2): who it was for and what became of it, and what the executor did, as a
 * timeline and as the log itself. The run and its log are read on their own: a log that cannot be
 * read does not take the rest of the page with it, and neither does the issue list (without it
 * there is just no link to Linear).
 */
export default function RunDetail() {
  const id = runId(useParams().id);
  // Keyed by the run: the tab chosen and the cards opened belong to one run, not to the next.
  return id === null ? <NotFound /> : <RunPage key={id} id={id} />;
}

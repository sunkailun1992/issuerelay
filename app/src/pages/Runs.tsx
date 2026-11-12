import { ChevronDown, ChevronRight, History } from "lucide-react";
import { Fragment, useId, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { Link, useNavigate, useSearchParams } from "react-router";
import { PageHeader } from "@/components/PageHeader";
import { RunDuration } from "@/components/RunDuration";
import { StatusBadge } from "@/components/StatusBadge";
import { DetailLine, EmptyState, ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { RUN_LIST_LIMIT, useJobs, useStatus } from "@/hooks/queries";
import { useLang } from "@/hooks/useLang";
import { useTick } from "@/hooks/useTick";
import { cleanDisplayText } from "@/lib/clean";
import { formatCost, formatRelative, formatTime } from "@/lib/format";
import { groupIssueRuns, type IssueRunGroup } from "@/lib/run-groups";
import { displayJobState, isPlanned, isPlanRun, parseRunFilter, RUN_FILTERS, type RunFilter } from "@/lib/runs";
import type { HeldJob, Job } from "@/lib/types";

/** What stands where a run has no value (it has no executor yet, or has not started). */
const NONE = "—";

/**
 * The columns, each with the keys of the facts it shows, by their names in `runs.field`. The
 * window can be as small as 900 px, which leaves the table about 600 px, so the state, the
 * issue, the title, the time and the note do not all fit as seven columns: the executor is
 * a second line under the identifier and the duration one under when it started.
 */
const COLUMNS = [["state"], ["issue", "executor"], ["title"], ["started", "duration"], ["note"]] as const;

/** How often "5 minutes ago" is worked out again. */
const CLOCK_MS = 30_000;

/** The first line of a text that has one worth showing. */
function firstLine(text: string): string {
  return text.split(/\r?\n/).find((line) => cleanDisplayText(line).trim() !== "") ?? "";
}

/**
 * What came of the run, in a line: when it waits to be tried again, that time; otherwise the AI's
 * summary (cleaned) or the engine's reason (a detail line), whichever the state calls for: a run
 * that failed or was stopped says why, one that did something says what.
 */
function Note({ job, held }: { job: Job; held: HeldJob | undefined }) {
  const { t } = useTranslation();
  const lang = useLang();
  if (job.state === "queued" && held !== undefined) {
    return <p className="text-muted-foreground">{t("state.retrying_at", { time: formatTime(held.retry_at, lang) })}</p>;
  }
  const summary = job.summary === null ? "" : cleanDisplayText(firstLine(job.summary)).trim();
  const reason = job.reason === null ? "" : firstLine(job.reason);
  const showReason = reason !== "" && (job.state === "failed" || job.state === "canceled" || summary === "");
  if (showReason) return <DetailLine text={reason} />;
  return summary === "" ? null : <p className="text-muted-foreground">{summary}</p>;
}

/**
 * One run. The whole row opens the run, and its identifier is the link for the keyboard and for
 * screen readers. A click that the link has already handled, or that ends a text selection (the
 * reason is text a reader may want to copy), is not a second one.
 */
function RunRow({ job, held, history = false, latest = false, historyToggle }: {
  job: Job;
  held: HeldJob | undefined;
  history?: boolean;
  latest?: boolean;
  historyToggle?: { count: number; expanded: boolean; id: string; toggle: () => void };
}) {
  const { t } = useTranslation();
  const lang = useLang();
  const navigate = useNavigate();
  const open = (event: MouseEvent) => {
    if (event.defaultPrevented || (window.getSelection()?.toString() ?? "") !== "") return;
    void navigate(`/runs/${job.id}`);
  };
  return (
    <TableRow className="cursor-pointer" onClick={open}>
      <TableCell>
        <StatusBadge state={job.state} planned={isPlanned(job)} />
      </TableCell>
      <TableCell>
        <div className="flex flex-col gap-0.5">
          <Link
            to={`/runs/${job.id}`}
            className="rounded-sm font-mono text-xs text-primary underline-offset-4 outline-none hover:underline focus-visible:ring-3 focus-visible:ring-ring/50"
          >
            {history ? t("runs.run_label", { id: job.id }) : job.issue_identifier}
          </Link>
          {job.executor !== null && (
            <span className="text-xs text-muted-foreground">
              {t(`executor.${job.executor}`)}
              {!history && isPlanRun(job) && ` · ${t("runs.mode.plan")}`}
            </span>
          )}
          {history && latest && <span className="text-xs text-muted-foreground">{t("runs.latest")}</span>}
        </div>
      </TableCell>
      <TableCell>
        <div className="flex max-w-sm flex-col items-start gap-1 wrap-anywhere whitespace-normal">
          {history ? (
            <>
              <span>{t(isPlanRun(job) ? "runs.mode.plan" : "runs.mode.implement")}</span>
              {job.resume_job_id != null && (
                <Link to={`/runs/${job.resume_job_id}`} className="text-xs text-primary underline-offset-4 hover:underline">
                  {t("runs.resumed_from", { id: job.resume_job_id })}
                </Link>
              )}
            </>
          ) : job.title}
          {historyToggle !== undefined && (
            <Button
              variant="ghost"
              size="xs"
              className="max-w-full whitespace-normal text-muted-foreground"
              aria-label={t(historyToggle.expanded ? "runs.collapse_history" : "runs.expand_history", { issue: job.issue_identifier })}
              aria-expanded={historyToggle.expanded}
              aria-controls={historyToggle.id}
              onClick={(event) => {
                event.stopPropagation();
                historyToggle.toggle();
              }}
            >
              {historyToggle.expanded ? <ChevronDown aria-hidden /> : <ChevronRight aria-hidden />}
              {t("runs.history_count", { count: historyToggle.count })}
            </Button>
          )}
        </div>
      </TableCell>
      <TableCell>
        {!history && job.started_at === null ? (
          NONE
        ) : (
          <div className="flex flex-col gap-0.5">
            {history && job.started_at === null && <span className="text-xs text-muted-foreground">{t("runs.field.created")}</span>}
            <span>{history ? formatTime(job.started_at ?? job.created_at, lang) : formatRelative(job.started_at!, lang)}</span>
            <span className="text-xs text-muted-foreground">
              <RunDuration job={job} />
              {job.usage?.cost_micro_usd != null && ` · ${formatCost(job.usage.cost_micro_usd)}`}
            </span>
          </div>
        )}
      </TableCell>
      <TableCell>
        <div className="max-w-xs whitespace-normal [&>p]:line-clamp-3 [&>p]:wrap-anywhere">
          <Note job={job} held={held} />
        </div>
      </TableCell>
    </TableRow>
  );
}

function IssueHistory({ group, held }: { group: IssueRunGroup; held: HeldJob[] }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const id = useId();
  return (
    <>
      <RunRow
        job={group.latest}
        held={held.find((pause) => pause.job_id === group.latest.id)}
        historyToggle={{ count: group.runs.length, expanded, id, toggle: () => setExpanded(!expanded) }}
      />
      {expanded && (
        <TableRow className="hover:bg-transparent">
          <TableCell colSpan={COLUMNS.length} className="bg-muted/30 p-2">
            <section id={id} aria-label={t("runs.history_caption", { issue: group.latest.issue_identifier })}>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("runs.field.state")}</TableHead>
                    <TableHead>{t("nav.runs")} / {t("runs.field.executor")}</TableHead>
                    <TableHead>{t("runs.field.mode")}</TableHead>
                    <TableHead>{t("runs.field.started")} / {t("runs.field.duration")}</TableHead>
                    <TableHead>{t("runs.field.note")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {group.runs.map((job) => (
                    <RunRow key={job.id} job={job} held={held.find((pause) => pause.job_id === job.id)} history latest={job.id === group.latest.id} />
                  ))}
                </TableBody>
              </Table>
            </section>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

function RunsTable({ groups, held }: { groups: IssueRunGroup[]; held: HeldJob[] }) {
  const { t } = useTranslation();
  // "5 minutes ago" stays right while the page is open.
  useTick(CLOCK_MS);
  return (
    <Table>
      <TableHeader>
        <TableRow>
          {COLUMNS.map((facts) => (
            <TableHead key={facts[0]}>
              {/* A header that has to wrap breaks between the names, never inside one. */}
              <span className="whitespace-normal">
                {facts.map((fact, index) => (
                  <Fragment key={fact}>
                    {index > 0 && " / "}
                    <span className="inline-block whitespace-nowrap">{t(`runs.field.${fact}`)}</span>
                  </Fragment>
                ))}
              </span>
            </TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>
        {groups.map((group) => (
          <IssueHistory key={group.key} group={group} held={held} />
        ))}
      </TableBody>
    </Table>
  );
}

/**
 * The runs (design 9.2), newest first, filtered by state with tabs whose choice is kept in the
 * address (`?state=`). Each issue shows its latest run; expanding it reveals the loaded history.
 * A state filter uses that latest run and keeps all its previous attempts together. The list is
 * the latest `RUN_LIST_LIMIT` runs, so it says when older attempts may have been left out.
 */
export default function Runs() {
  const { t } = useTranslation();
  const lang = useLang();
  const [params, setParams] = useSearchParams();
  const filter: RunFilter = parseRunFilter(params.get("state"));
  const jobs = useJobs();
  const status = useStatus();

  const header = <PageHeader title={t("nav.runs")} />;
  if (jobs.isError && !jobs.isFetching) {
    return (
      <>
        {header}
        <ErrorState error={jobs.error} headline="runs.load_failed" onRetry={() => void jobs.refetch()} />
      </>
    );
  }

  const shown = jobs.data === undefined ? undefined : groupIssueRuns(jobs.data)
    .filter((group) => filter === "all" || displayJobState(group.latest.state) === filter);
  let content;
  if (shown === undefined) {
    content = <LoadingState rows={5} />;
  } else if (jobs.data?.length === 0) {
    content = <EmptyState icon={History} title={t("runs.empty")} description={t("runs.empty_description")} />;
  } else if (shown.length === 0) {
    content = <EmptyState icon={History} title={t("runs.empty_filtered")} description={t("runs.empty_filtered_description")} />;
  } else {
    // A queued run that failed to start waits for a retry: the status knows when.
    content = <RunsTable groups={shown} held={status.data?.status.held ?? []} />;
  }

  return (
    <>
      {header}
      <Tabs value={filter} onValueChange={(value) => setParams({ state: parseRunFilter(value) }, { replace: true })}>
        {/* The state tabs wrap in a narrow window. */}
        <TabsList className="h-auto! flex-wrap justify-start gap-1">
          {RUN_FILTERS.map((value) => (
            <TabsTrigger key={value} value={value} className="flex-none!">
              {value === "all" ? t("runs.all") : t(`state.${value}`)}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value={filter} className="flex flex-col gap-3 pt-2">
          {jobs.data !== undefined && jobs.data.length >= RUN_LIST_LIMIT && (
            <p className="text-sm text-muted-foreground">{t("runs.list_capped", { runs: new Intl.NumberFormat(lang).format(RUN_LIST_LIMIT) })}</p>
          )}
          {content}
        </TabsContent>
      </Tabs>
    </>
  );
}

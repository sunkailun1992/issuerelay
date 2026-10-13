import { History } from "lucide-react";
import { Fragment, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { Link, useNavigate, useSearchParams } from "react-router";
import { PageHeader } from "@/components/PageHeader";
import { RunDuration } from "@/components/RunDuration";
import { StatusBadge } from "@/components/StatusBadge";
import { DetailLine, EmptyState, ErrorState, LoadingState } from "@/components/states";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useJobs, useStatus } from "@/hooks/queries";
import { useLang } from "@/hooks/useLang";
import { useTick } from "@/hooks/useTick";
import { cleanDisplayText } from "@/lib/clean";
import { formatRelative, formatTime } from "@/lib/format";
import { parseRunFilter, RUN_FILTERS, type RunFilter } from "@/lib/runs";
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
function RunRow({ job, held }: { job: Job; held: HeldJob | undefined }) {
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
        <StatusBadge state={job.state} />
      </TableCell>
      <TableCell>
        <div className="flex flex-col gap-0.5">
          <Link
            to={`/runs/${job.id}`}
            className="rounded-sm font-mono text-xs text-primary underline-offset-4 outline-none hover:underline focus-visible:ring-3 focus-visible:ring-ring/50"
          >
            {job.issue_identifier}
          </Link>
          {job.executor !== null && <span className="text-xs text-muted-foreground">{t(`executor.${job.executor}`)}</span>}
        </div>
      </TableCell>
      <TableCell>
        <div className="max-w-sm wrap-anywhere whitespace-normal">{job.title}</div>
      </TableCell>
      <TableCell>
        {job.started_at === null ? (
          NONE
        ) : (
          <div className="flex flex-col gap-0.5">
            <span>{formatRelative(job.started_at, lang)}</span>
            <span className="text-xs text-muted-foreground">
              <RunDuration job={job} />
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

function RunsTable({ jobs, held }: { jobs: Job[]; held: HeldJob[] }) {
  const { t } = useTranslation();
  // "5 minutes ago" stays right while the page is open.
  useTick(CLOCK_MS);
  return (
    <Table>
      <TableHeader>
        <TableRow>
          {COLUMNS.map((facts) => (
            <TableHead key={facts[0]}>
              {/* A header that has to wrap breaks between the names, not inside one. */}
              <span className="whitespace-normal">
                {facts.map((fact, index) => (
                  <Fragment key={fact}>
                    {index > 0 && " / "}
                    <span className="inline-block">{t(`runs.field.${fact}`)}</span>
                  </Fragment>
                ))}
              </span>
            </TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>
        {jobs.map((job) => (
          <RunRow key={job.id} job={job} held={held.find((pause) => pause.job_id === job.id)} />
        ))}
      </TableBody>
    </Table>
  );
}

/**
 * The runs (design 9.2), newest first, filtered by state with tabs whose choice is kept in the
 * address (`?state=`, so the overview's "View" for failed runs lands on the right tab). A row opens
 * the run's page.
 */
export default function Runs() {
  const { t } = useTranslation();
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

  const shown = jobs.data?.filter((job) => filter === "all" || job.state === filter);
  let content;
  if (shown === undefined) {
    content = <LoadingState rows={5} />;
  } else if (jobs.data?.length === 0) {
    content = <EmptyState icon={History} title={t("runs.empty")} description={t("runs.empty_description")} />;
  } else if (shown.length === 0) {
    content = <EmptyState icon={History} title={t("runs.empty_filtered")} description={t("runs.empty_filtered_description")} />;
  } else {
    // A queued run that failed to start waits for a retry: the status knows when.
    content = <RunsTable jobs={shown} held={status.data?.status.held ?? []} />;
  }

  return (
    <>
      {header}
      <Tabs value={filter} onValueChange={(value) => setParams({ state: parseRunFilter(value) }, { replace: true })}>
        {/* Nine tabs do not fit a narrow window in one line: they wrap. */}
        <TabsList className="h-auto! flex-wrap justify-start gap-1">
          {RUN_FILTERS.map((value) => (
            <TabsTrigger key={value} value={value} className="flex-none!">
              {value === "all" ? t("runs.all") : t(`state.${value}`)}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value={filter} className="pt-2">
          {content}
        </TabsContent>
      </Tabs>
    </>
  );
}

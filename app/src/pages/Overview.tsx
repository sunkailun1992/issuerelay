import { Activity, CircleX, ClipboardCheck, Hourglass, RefreshCw, TriangleAlert, Wrench } from "lucide-react";
import { type ReactNode, useId } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { JobActions } from "@/components/JobActions";
import { LinearStatus } from "@/components/LinearStatus";
import { PageHeader } from "@/components/PageHeader";
import { StatusBadge } from "@/components/StatusBadge";
import { DetailLine, EmptyState, ErrorState, LoadingState } from "@/components/states";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { DOCTOR_FRESH_MS, useBindings, useDoctor, useJobs, usePause, usePollNow, useResume, useStatus } from "@/hooks/queries";
import { useLang } from "@/hooks/useLang";
import { useTick } from "@/hooks/useTick";
import { cleanDisplayText } from "@/lib/clean";
import { formatDuration, formatRelative, formatTime } from "@/lib/format";
import { problemCause } from "@/lib/problem";
import { isPlanned, newestRuns } from "@/lib/runs";
import type { HeldJob, Job, JobState, LinearSetup, StatusView } from "@/lib/types";
import { cn } from "@/lib/utils";
import { executorReady } from "./onboarding/check";

/** How many finished runs the overview lists. */
const RECENT_COUNT = 8;
/** The states of a run that has ended. */
const FINISHED: readonly JobState[] = ["awaiting_acceptance", "needs_info", "done", "failed", "canceled"];
/** The states of a run that is on its way to start. */
const WAITING: readonly JobState[] = ["queued", "waiting_binding"];

/** How often a duration is worked out again, and how often "5 minutes ago". */
const SECOND_MS = 1000;
const CLOCK_MS = 30_000;

/** Check now and the monitoring switch, on the right of the header. */
function Controls({ paused }: { paused: boolean }) {
  const { t } = useTranslation();
  const id = useId();
  const poll = usePollNow();
  const pause = usePause();
  const resume = useResume();
  return (
    <>
      <Button variant="outline" size="sm" disabled={poll.isPending} onClick={() => poll.mutate()}>
        <RefreshCw aria-hidden className={cn(poll.isPending && "animate-spin")} />
        {t("actions.poll_now")}
      </Button>
      <div className="flex items-center gap-2">
        <Switch
          id={id}
          checked={!paused}
          disabled={pause.isPending || resume.isPending}
          onCheckedChange={(monitoring) => (monitoring ? resume.mutate() : pause.mutate())}
        />
        <Label htmlFor={id}>{t("overview.monitoring")}</Label>
      </div>
    </>
  );
}

/** The link at the end of a reminder's title row. */
const VIEW_LINK = "text-sm underline underline-offset-3 hover:text-foreground";

/** The engine's own words about a problem, if it has any: Linear's message or the last error. */
function problemDetail({ status, linear }: StatusView): string | null {
  return linear.kind === "connect_failed" || linear.kind === "interface_changed" ? linear.message : status.last_error;
}

/**
 * What needs the user: an environment problem (the tray's rule), runs whose newest try failed,
 * issues that await acceptance, and the wait for the runs in progress to end. `jobs` is not
 * there while the runs are being read (or cannot be): only the reminders that need them go.
 * Only the problem is an alert, which a screen reader announces at once; the others are notices
 * (`role="status"`, announced politely). An icon takes its state colour with an important
 * utility (`!` at the end): the alert sets its icon to `text-current` and would win otherwise.
 */
function Reminders({ view, jobs }: { view: StatusView; jobs: Job[] | undefined }) {
  const { t } = useTranslation();
  const cause = problemCause(view.status, view.linear);
  const detail = problemDetail(view);
  // A failure that a later run of the same issue followed is history, not a reminder.
  const latest = jobs === undefined ? [] : newestRuns(jobs);
  const failed = latest.filter((job) => job.state === "failed").length;
  const awaiting = latest.filter((job) => job.state === "awaiting_acceptance").length;
  if (cause === null && failed === 0 && awaiting === 0 && !view.status.draining) return null;
  return (
    <div className="flex flex-col gap-3">
      {cause !== null && (
        <Alert variant="destructive">
          <TriangleAlert aria-hidden />
          <AlertTitle>{t(`overview.problem.${cause}`)}</AlertTitle>
          <AlertDescription>
            {detail !== null && (
              <div>
                <DetailLine text={detail} />
              </div>
            )}
            <div>
              <Link to="/doctor">{t("overview.open_doctor")}</Link>
            </div>
          </AlertDescription>
        </Alert>
      )}
      {failed > 0 && (
        <Alert role="status">
          <CircleX aria-hidden className="text-state-failed!" />
          <AlertTitle>{t("overview.alert_failed", { count: failed })}</AlertTitle>
          <AlertAction>
            <Link to="/runs?state=failed" className={VIEW_LINK}>
              {t("overview.view")}
            </Link>
          </AlertAction>
        </Alert>
      )}
      {awaiting > 0 && (
        <Alert role="status">
          <ClipboardCheck aria-hidden className="text-state-awaiting!" />
          <AlertTitle>{t("overview.alert_awaiting", { count: awaiting })}</AlertTitle>
          <AlertAction>
            <Link to="/issues" className={VIEW_LINK}>
              {t("overview.view")}
            </Link>
          </AlertAction>
        </Alert>
      )}
      {view.status.draining && (
        <Alert role="status">
          <Hourglass aria-hidden />
          <AlertTitle>{t("overview.draining")}</AlertTitle>
        </Alert>
      )}
    </div>
  );
}

/**
 * The next step when the page has none to show (Linear connected, no executor ready, nothing bound,
 * so nothing can run and nothing tells why): set up an executor, then bind a project. The
 * environment check runs the executors, so it is asked for only then, and an answer younger than
 * `DOCTOR_FRESH_MS` is taken as it is. A check that is still running or failed shows nothing.
 */
function SetupNote({ linear }: { linear: LinearSetup }) {
  const { t } = useTranslation();
  const bindings = useBindings();
  const unbound = linear.kind === "connected" && bindings.data !== undefined && bindings.data.length === 0;
  const doctor = useDoctor(unbound, { staleTime: DOCTOR_FRESH_MS });
  if (!unbound || doctor.data === undefined || doctor.data.executors.some(executorReady)) return null;
  return (
    <Alert role="status">
      <Wrench aria-hidden />
      <AlertTitle>{t("overview.setup_title")}</AlertTitle>
      <AlertDescription>
        <p>{t("overview.setup_body")}</p>
        <div className="flex flex-wrap gap-x-4 gap-y-1">
          <Link to="/doctor">{t("overview.open_doctor")}</Link>
          <Link to="/bindings">{t("overview.bind_project")}</Link>
        </div>
      </AlertDescription>
    </Alert>
  );
}

/** A block of the page: its name is the heading, which makes it a region of its own. */
function Section({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="flex flex-col gap-3">
      <h2 id={id} className="text-base font-semibold">
        {title}
      </h2>
      {children}
    </section>
  );
}

/** The frame of a list of rows. */
const LIST = "divide-y rounded-xl border bg-card text-card-foreground";

/** A run in progress, with how long it has been going: counted again every second. */
function RunningRow({ job }: { job: Job }) {
  const { t } = useTranslation();
  const lang = useLang();
  useTick(SECOND_MS);
  const facts = [
    job.executor === null ? null : t(`executor.${job.executor}`),
    job.started_at === null ? null : formatDuration(job.started_at, null, lang),
  ].filter((fact) => fact !== null);
  return (
    <li className="flex items-center justify-between gap-4 px-4 py-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <Link to={`/runs/${job.id}`} className="flex min-w-0 items-baseline gap-2 hover:underline">
          <span className="shrink-0 font-mono text-xs text-muted-foreground">{job.issue_identifier}</span>
          <span className="truncate font-medium">{job.title}</span>
        </Link>
        <p className="text-xs text-muted-foreground">{facts.join(" · ")}</p>
      </div>
      <JobActions job={job} compact />
    </li>
  );
}

function RunningSection({ jobs }: { jobs: Job[] }) {
  const { t } = useTranslation();
  const running = jobs.filter((job) => job.state === "running");
  return (
    <Section title={t("overview.running")}>
      {running.length === 0 ? (
        // The empty state is made for a whole page; here it sits between two blocks.
        <div className="rounded-xl border bg-card [&>div]:py-8">
          <EmptyState icon={Activity} title={t("overview.no_running")} description={t("overview.no_running_hint")} />
        </div>
      ) : (
        <ul className={LIST}>
          {running.map((job) => (
            <RunningRow key={job.id} job={job} />
          ))}
        </ul>
      )}
    </Section>
  );
}

/** The room for a state's badge: the widest ("Awaiting acceptance") fits, so the numbers and titles of all rows line up. */
function StateColumn({ job }: { job: Job }) {
  return (
    <div className="w-44 shrink-0">
      <StatusBadge state={job.state} planned={isPlanned(job)} />
    </div>
  );
}

/** A run that has not started: queued (and maybe held back after a failed start), or waiting for a binding. */
function QueueRow({ job, held }: { job: Job; held: HeldJob | undefined }) {
  const { t } = useTranslation();
  const lang = useLang();
  return (
    <li className="flex items-start gap-3 px-4 py-3">
      <StateColumn job={job} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <p className="flex min-w-0 flex-wrap items-baseline gap-x-2">
          <span className="font-mono text-xs text-muted-foreground">{job.issue_identifier}</span>
          <span className="font-medium">{job.title}</span>
          {held !== undefined && (
            <span className="text-xs text-muted-foreground">{t("state.retrying_at", { time: formatTime(held.retry_at, lang) })}</span>
          )}
        </p>
        {/* The engine's sentence under the translated state, never instead of it. */}
        {job.state === "waiting_binding" && job.reason !== null && <DetailLine text={job.reason} />}
      </div>
    </li>
  );
}

function QueueSection({ jobs, held }: { jobs: Job[]; held: HeldJob[] }) {
  const { t } = useTranslation();
  // The runs that will start first, then those that wait for something; the oldest first.
  const queue = jobs
    .filter((job) => WAITING.includes(job.state))
    .sort((a, b) => WAITING.indexOf(a.state) - WAITING.indexOf(b.state) || a.id - b.id);
  return (
    <Section title={t("overview.queued")}>
      {queue.length === 0 ? (
        <p className="text-muted-foreground">{t("overview.queue_empty")}</p>
      ) : (
        <ul className={LIST}>
          {queue.map((job) => (
            <QueueRow key={job.id} job={job} held={held.find((pause) => pause.job_id === job.id)} />
          ))}
        </ul>
      )}
    </Section>
  );
}

/** The first line of a text that has any, as the engine or the AI wrote it. */
function firstLine(text: string): string {
  return text.split(/\r?\n/).find((line) => cleanDisplayText(line).trim() !== "") ?? "";
}

/** When a run ended, for ordering; a run without an end time counts from when it was recorded. */
function endedAt(job: Job): number {
  return Date.parse(job.finished_at ?? job.created_at);
}

function RecentRow({ job }: { job: Job }) {
  const lang = useLang();
  // The AI's summary is its own words, cleaned; a reason is the engine's sentence, a detail line.
  const summary = job.summary === null ? "" : cleanDisplayText(firstLine(job.summary)).trim();
  const reason = job.reason === null ? "" : firstLine(job.reason);
  // What matters about a run that did not end well is why; about the others, what was done.
  const showReason = reason !== "" && (job.state === "failed" || job.state === "canceled" || summary === "");
  return (
    <li>
      <Link
        to={`/runs/${job.id}`}
        className="flex items-start gap-3 px-4 py-3 outline-none hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50"
      >
        <StateColumn job={job} />
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <p className="flex min-w-0 items-baseline gap-2">
            <span className="shrink-0 font-mono text-xs text-muted-foreground">{job.issue_identifier}</span>
            <span className="truncate font-medium">{job.title}</span>
          </p>
          {showReason ? <DetailLine text={reason} /> : summary !== "" ? <p className="truncate text-muted-foreground">{summary}</p> : null}
        </div>
        {job.finished_at !== null && <span className="shrink-0 text-xs text-muted-foreground">{formatRelative(job.finished_at, lang)}</span>}
      </Link>
    </li>
  );
}

function RecentSection({ jobs }: { jobs: Job[] }) {
  const { t } = useTranslation();
  // "5 minutes ago" stays right while the page is open.
  useTick(CLOCK_MS);
  const recent = jobs
    .filter((job) => FINISHED.includes(job.state))
    .sort((a, b) => endedAt(b) - endedAt(a) || b.id - a.id)
    .slice(0, RECENT_COUNT);
  return (
    <Section title={t("overview.recent")}>
      {recent.length === 0 ? (
        <p className="text-muted-foreground">{t("overview.recent_empty")}</p>
      ) : (
        <ul className={LIST}>
          {recent.map((job) => (
            <RecentRow key={job.id} job={job} />
          ))}
        </ul>
      )}
    </Section>
  );
}

/**
 * The overview (design 9.2): the monitoring switch, how Linear stands, what needs the user, and
 * what runs, waits and has just finished. Everything comes from the status and the list of runs,
 * which the engine's events keep current; a status that cannot be read replaces the page, runs
 * that cannot be read replace only their three blocks.
 */
export default function Overview() {
  const { t } = useTranslation();
  const status = useStatus();
  const jobs = useJobs();

  if (status.isError && !status.isFetching) {
    return (
      <>
        <PageHeader title={t("nav.overview")} />
        <ErrorState error={status.error} headline="overview.status_failed" onRetry={() => void status.refetch()} />
      </>
    );
  }

  return (
    <>
      <PageHeader title={t("nav.overview")} actions={status.data && <Controls paused={status.data.status.paused} />} />
      <div className="flex flex-col gap-6">
        {status.data === undefined ? (
          <LoadingState rows={2} />
        ) : (
          <>
            <LinearStatus />
            <SetupNote linear={status.data.linear} />
            <Reminders view={status.data} jobs={jobs.data} />
          </>
        )}
        {jobs.isError && !jobs.isFetching ? (
          <ErrorState error={jobs.error} headline="overview.jobs_failed" onRetry={() => void jobs.refetch()} />
        ) : jobs.data === undefined ? (
          <LoadingState />
        ) : (
          <>
            <RunningSection jobs={jobs.data} />
            <QueueSection jobs={jobs.data} held={status.data?.status.held ?? []} />
            <RecentSection jobs={jobs.data} />
          </>
        )}
      </div>
    </>
  );
}

import type { TFunction } from "i18next";
import { Inbox, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { IssueLink } from "@/components/IssueLink";
import { JobActions } from "@/components/JobActions";
import { PageHeader } from "@/components/PageHeader";
import { StatusBadge } from "@/components/StatusBadge";
import { EmptyState, ErrorState, LinearConnecting, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useIssues, useStatus } from "@/hooks/queries";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import { cleanDisplayText } from "@/lib/clean";
import { isActive, isPlanned, isPlanRun } from "@/lib/runs";
import { sameName } from "@/lib/settings-form";
import type { IssueStatus, IssueView, TriggerNames } from "@/lib/types";
import { cn } from "@/lib/utils";

/** Whether an issue's labels ask for a planning run: the plan label without the trigger label (core's `candidate_mode`). */
function asksForPlan(labels: readonly string[], names: TriggerNames): boolean {
  const carries = (wanted: string) => labels.some((label) => sameName(label, wanted));
  return carries(names.plan_label) && !carries(names.label);
}

/** The columns, by the key of their name in `issues.column`. */
const COLUMNS = ["issue", "title", "linear_state", "local_state", "reason", "actions"] as const;

/**
 * What the engine made of an issue, in words. The engine sends a verdict and, for an issue that
 * waits, a code; the sentence is the panel's own, so it is in the interface language.
 */
function describeStatus(status: IssueStatus, names: TriggerNames, t: TFunction): string {
  switch (status.kind) {
    case "eligible":
      return status.picked_up ? t("issues.eligible") : t("issues.runs_again_after_change");
    case "not_todo":
      return t("issues.not_todo", { state: status.state, todo: names.todo });
    case "not_candidate":
      return t("issues.not_candidate");
    case "paused":
      return t("issues.paused");
    case "waiting":
      switch (status.reason.code) {
        case "project_not_bound":
          return t("issues.waiting.project_not_bound");
        case "directory_missing":
          return t("issues.waiting.directory_missing", { directory: cleanDisplayText(status.reason.directory).trim() });
        case "executor_not_ready":
          return t("issues.waiting.executor_not_ready", { executor: t(`executor.${status.reason.executor}`) });
        case "lean_not_ready":
          return t("issues.waiting.lean_not_ready", { executor: t(`executor.${status.reason.executor}`) });
        case "blocked_by":
          return t("issues.waiting.blocked_by", { issues: status.reason.issues.map((issue) => cleanDisplayText(issue).trim()).join(", ") });
        case "labels_match_several_bindings":
          return t("issues.waiting.labels_match_several_bindings", {
            labels: status.reason.labels.map((label) => cleanDisplayText(label).trim()).join(", "),
          });
        case "no_binding_for_labels":
          return t("issues.waiting.no_binding_for_labels");
        case "plan_needs_lean":
          return t("issues.waiting.plan_needs_lean");
      }
  }
}

/**
 * One issue. The window can be as small as 900 px, which leaves the table about 600 px (a little
 * less where the scrollbar takes room), and a row has to fit: the identifier is the link to Linear
 * (`IssueLink`), so the actions are one button at most; the title and the reason wrap, a long word
 * of them too (a folder's path, say); a state name wraps between words.
 */
function IssueRow({ view, names }: { view: IssueView; names: TriggerNames }) {
  const { t } = useTranslation();
  const { issue, job, status } = view;
  // While a run is in hand its badge says it all; the issue's status is what holds back a run that
  // has ended or has not begun.
  const reason = status.kind === "not_todo" && job !== null && isActive(job) ? null : describeStatus(status, names, t);
  const pickedUp = status.kind === "eligible" && status.picked_up;
  return (
    <TableRow>
      <TableCell>
        {/* items-start: the link keeps its own width, as it had alone in the cell. */}
        <div className="flex flex-col items-start gap-0.5">
          <IssueLink identifier={issue.identifier} url={issue.url} />
          {/* What the next run is for: the run in hand's, else what the labels ask for. */}
          {(job !== null && isActive(job) ? isPlanRun(job) : asksForPlan(issue.labels, names)) && (
            <span className="text-xs text-muted-foreground">{t("runs.mode.plan")}</span>
          )}
        </div>
      </TableCell>
      <TableCell>
        <div className="max-w-sm wrap-anywhere whitespace-normal">{issue.title}</div>
      </TableCell>
      <TableCell>
        {/* A state name is a short word or two: it wraps between words, never inside one. */}
        <div className="whitespace-normal">{issue.state}</div>
      </TableCell>
      <TableCell>
        {job === null ? <span className="text-muted-foreground">—</span> : <StatusBadge state={job.state} planned={isPlanned(job)} />}
      </TableCell>
      <TableCell>
        {reason !== null && <div className={cn("max-w-xs wrap-anywhere whitespace-normal", !pickedUp && "text-muted-foreground")}>{reason}</div>}
      </TableCell>
      <TableCell>
        <JobActions job={job} compact />
      </TableCell>
    </TableRow>
  );
}

/**
 * The issues the engine would look at (design 9.2): assigned to the user, with the trigger label,
 * newest update first, each with its newest run and whether, and if not why, the next check picks
 * it up. The list is read from Linear, so it can fail on its own: then the page says what to do.
 * While the first connection to Linear is under way it waits for it instead.
 */
export default function Issues() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const issues = useIssues();
  const status = useStatus();
  const names = useTriggerNames();

  const header = (
    <PageHeader
      title={t("nav.issues")}
      description={t("issues.description", { label: names.label, plan_label: names.plan_label })}
      actions={
        <Button variant="outline" size="sm" disabled={issues.isFetching} onClick={() => void issues.refetch()}>
          <RefreshCw aria-hidden className={cn(issues.isFetching && "animate-spin")} />
          {t("common.refresh")}
        </Button>
      }
    />
  );

  if (status.data?.linear.kind === "connecting") {
    return (
      <>
        {header}
        <LinearConnecting />
      </>
    );
  }
  if (issues.isError && !issues.isFetching) {
    return (
      <>
        {header}
        <div className="flex flex-col gap-3">
          <ErrorState error={issues.error} headline="issues.load_failed" onRetry={() => void issues.refetch()} />
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" onClick={() => navigate("/onboarding")}>
              {t("issues.open_guide")}
            </Button>
            <Button variant="outline" size="sm" onClick={() => navigate("/doctor")}>
              {t("issues.open_doctor")}
            </Button>
          </div>
        </div>
      </>
    );
  }
  if (issues.data === undefined) {
    return (
      <>
        {header}
        <LoadingState rows={5} />
      </>
    );
  }
  if (issues.data.length === 0) {
    return (
      <>
        {header}
        <EmptyState
          icon={Inbox}
          title={t("issues.empty_title", { label: names.label })}
          description={t("issues.empty_description", { label: names.label, todo: names.todo })}
          action={
            <Button variant="outline" size="sm" onClick={() => navigate("/help")}>
              {t("issues.open_help")}
            </Button>
          }
        />
      </>
    );
  }
  return (
    <>
      {header}
      <Table>
        <TableHeader>
          <TableRow>
            {/* A column's name keeps to one line (head cells do not wrap): a Chinese one would otherwise break between its characters. */}
            {COLUMNS.map((column) => (
              <TableHead key={column}>{t(`issues.column.${column}`)}</TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {issues.data.map((view) => (
            <IssueRow key={view.issue.uuid} view={view} names={names} />
          ))}
        </TableBody>
      </Table>
    </>
  );
}

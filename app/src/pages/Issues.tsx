import type { TFunction } from "i18next";
import { Inbox, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { JobActions } from "@/components/JobActions";
import { PageHeader } from "@/components/PageHeader";
import { StatusBadge } from "@/components/StatusBadge";
import { EmptyState, ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useIssues } from "@/hooks/queries";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import { cleanDisplayText } from "@/lib/clean";
import type { IssueStatus, IssueView, TriggerNames } from "@/lib/types";
import { cn } from "@/lib/utils";

/** The columns, by the key of their name in `issues.column`. */
const COLUMNS = ["issue", "title", "linear_state", "local_state", "reason", "actions"] as const;

/**
 * What the engine made of an issue, in words. The engine sends a verdict and, for an issue that
 * waits, a code; the sentence is the panel's own, so it is in the interface language.
 */
function describeStatus(status: IssueStatus, names: TriggerNames, t: TFunction): string {
  switch (status.kind) {
    case "eligible":
      return t("issues.eligible");
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
      }
  }
}

function IssueRow({ view, names }: { view: IssueView; names: TriggerNames }) {
  const { t } = useTranslation();
  const { issue, job, status } = view;
  return (
    <TableRow>
      <TableCell className="font-mono text-xs">{issue.identifier}</TableCell>
      <TableCell>
        <div className="max-w-sm whitespace-normal">{issue.title}</div>
      </TableCell>
      <TableCell>{issue.state}</TableCell>
      <TableCell>{job === null ? <span className="text-muted-foreground">—</span> : <StatusBadge state={job.state} />}</TableCell>
      <TableCell>
        <div className={cn("max-w-xs whitespace-normal", status.kind !== "eligible" && "text-muted-foreground")}>
          {describeStatus(status, names, t)}
        </div>
      </TableCell>
      <TableCell>
        <JobActions job={job} url={issue.url} compact />
      </TableCell>
    </TableRow>
  );
}

/**
 * The issues the engine would look at (design 9.2): assigned to the user, with the trigger label,
 * newest update first, each with its newest run and whether, and if not why, the next check picks
 * it up. The list is read from Linear, so it can fail on its own: then the page says what to do.
 */
export default function Issues() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const issues = useIssues();
  const names = useTriggerNames();

  const header = (
    <PageHeader
      title={t("nav.issues")}
      description={t("issues.description", { label: names.label })}
      actions={
        <Button variant="outline" size="sm" disabled={issues.isFetching} onClick={() => void issues.refetch()}>
          <RefreshCw aria-hidden className={cn(issues.isFetching && "animate-spin")} />
          {t("common.refresh")}
        </Button>
      }
    />
  );

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

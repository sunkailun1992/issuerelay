import { FolderGit2, LoaderCircle, RefreshCw, Unplug } from "lucide-react";
import { useDeferredValue, useId } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { CheckRow } from "@/components/CheckRow";
import { ExecutorCheck } from "@/components/ExecutorCheck";
import { LinearStatus } from "@/components/LinearStatus";
import { PageHeader } from "@/components/PageHeader";
import { Section } from "@/components/Section";
import { DetailLine, EmptyState, ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { WorkspaceCheck } from "@/components/WorkspaceCheck";
import { useDoctor } from "@/hooks/queries";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import { maskCredentials } from "@/lib/mask";
import type { BindingReport, DoctorReport, ExecEnv } from "@/lib/types";
import { cn } from "@/lib/utils";

/** What the check is doing, while it does it: it runs the executors and asks Linear, which takes up to a minute. */
function Checking() {
  const { t } = useTranslation();
  return (
    <p role="status" className="flex items-center gap-2 text-muted-foreground">
      <LoaderCircle aria-hidden className="size-4 shrink-0 animate-spin" />
      {t("doctor.checking")}
    </p>
  );
}

/** Where the workspace could not be looked at: not yet connected to Linear. */
function WorkspaceUnchecked() {
  const { t } = useTranslation();
  return (
    <p className="flex items-center gap-2 rounded-xl border border-dashed bg-card p-4 text-muted-foreground">
      <Unplug aria-hidden className="size-4 shrink-0" />
      {t("doctor.workspace_unchecked")}
    </p>
  );
}

/**
 * The PATH and the proxy the executors get, and where each was read from (design 8, last
 * paragraph): an app started from the Dock, Finder or at login has only a minimal PATH, and
 * Claude Code reads the proxy only from environment variables, so IssueRelay reads both and hands
 * them on. The proxy values are shown masked: the engine masks them, and so does the panel once
 * more.
 */
function EnvironmentCheck({ env }: { env: ExecEnv | null }) {
  const { t } = useTranslation();
  const why = <p className="text-muted-foreground">{t("doctor.env_why")}</p>;
  if (env === null) {
    return (
      <Card size="sm">
        <CardContent>
          <p>{t("doctor.env_inherited")}</p>
          {why}
        </CardContent>
      </Card>
    );
  }
  const noProxy = env.proxy_source === "none" && env.proxy.length === 0;
  return (
    <Card size="sm">
      <CardContent>
        <dl className="grid grid-cols-[max-content_minmax(0,1fr)] items-baseline gap-x-4 gap-y-2">
          <dt className="text-muted-foreground">{t("doctor.path_source")}</dt>
          <dd>{t(`doctor.source.${env.path_source}`)}</dd>
          {noProxy ? (
            <>
              <dt className="text-muted-foreground">{t("doctor.proxy")}</dt>
              <dd>{t("doctor.no_proxy")}</dd>
            </>
          ) : (
            <>
              <dt className="text-muted-foreground">{t("doctor.proxy_source")}</dt>
              <dd>{t(`doctor.source.${env.proxy_source}`)}</dd>
              <dt className="text-muted-foreground">{t("doctor.proxy_variables")}</dt>
              <dd className="min-w-0">
                {env.proxy.length === 0 ? (
                  t("doctor.no_variables")
                ) : (
                  <ul className="flex flex-col gap-1">
                    {env.proxy.map(([name, value]) => (
                      <li key={name} className="font-mono text-xs break-all select-text">
                        {`${name}=${maskCredentials(value)}`}
                      </li>
                    ))}
                  </ul>
                )}
              </dd>
            </>
          )}
        </dl>
        {env.problems.length > 0 && (
          <div className="flex flex-col gap-1">
            <p className="font-medium text-destructive">{t("doctor.env_problems")}</p>
            {env.problems.map((problem, index) => (
              <DetailLine key={index} text={problem} />
            ))}
          </div>
        )}
        {why}
      </CardContent>
    </Card>
  );
}

/** One binding as the check found it: its folder and its executor. */
function BindingCheck({ report }: { report: BindingReport }) {
  const { t } = useTranslation();
  const titleId = useId();
  return (
    <section aria-labelledby={titleId}>
      <Card size="sm" className="h-full">
        <div className="px-(--card-spacing)">
          <h3 id={titleId} className="text-base font-medium break-words">
            {report.project_name}
          </h3>
        </div>
        <CardContent>
          <ul className="flex flex-col gap-1.5">
            <CheckRow ok={report.directory_exists}>
              <span>{report.directory_exists ? t("bindings.directory_ok") : t("bindings.directory_missing")}</span>
            </CheckRow>
            <CheckRow ok={report.executor_ready}>
              <span>
                {t(report.executor_ready ? "bindings.executor_ready" : "bindings.executor_not_ready", {
                  executor: t(`executor.${report.executor}`),
                })}
              </span>
            </CheckRow>
          </ul>
        </CardContent>
      </Card>
    </section>
  );
}

function BindingsCheck({ bindings }: { bindings: BindingReport[] }) {
  const { t } = useTranslation();
  if (bindings.length === 0) {
    return (
      // The empty state is made for a whole page; here it sits between two blocks.
      <div className="rounded-xl border bg-card [&>div]:py-8">
        <EmptyState
          icon={FolderGit2}
          title={t("doctor.no_bindings")}
          description={t("bindings.empty_description")}
          action={
            <Button asChild variant="outline" size="sm">
              <Link to="/bindings">{t("doctor.open_bindings")}</Link>
            </Button>
          }
        />
      </div>
    );
  }
  return (
    <div className="grid grid-cols-[repeat(auto-fit,minmax(16rem,1fr))] gap-4">
      {bindings.map((binding) => (
        <BindingCheck key={binding.binding_id} report={binding} />
      ))}
    </div>
  );
}

/** What the check found, in its five blocks. */
function Report({ report, labelName }: { report: DoctorReport; labelName: string }) {
  const { t } = useTranslation();
  return (
    <>
      <Section title={t("doctor.linear_title")}>
        {/* The one card of the connection, here with what the check adds: authorizing again and logging out. */}
        <LinearStatus manage />
        {report.linear_error !== null && (
          <div className="flex flex-col gap-1">
            <p className="font-medium text-destructive">{t("doctor.linear_check_failed")}</p>
            <DetailLine text={report.linear_error} />
          </div>
        )}
      </Section>
      <Section title={t("doctor.workspace_title")}>
        {report.workspace === null ? <WorkspaceUnchecked /> : <WorkspaceCheck report={report.workspace} labelName={labelName} />}
      </Section>
      <Section title={t("doctor.executors_title")}>
        {/* One column at the smallest window, two where there is room. */}
        <div className="grid grid-cols-[repeat(auto-fit,minmax(20rem,1fr))] gap-4">
          {report.executors.map((executor) => (
            <ExecutorCheck key={executor.kind} report={executor} />
          ))}
        </div>
      </Section>
      <Section title={t("doctor.env_title")}>
        <EnvironmentCheck env={report.env} />
      </Section>
      <Section title={t("doctor.bindings_title")}>
        <BindingsCheck bindings={report.bindings} />
      </Section>
    </>
  );
}

/**
 * The environment check (design 8): Linear, the workspace, the executors, the environment they
 * run in and the bindings, each with what to do about what is wrong. Opening the page runs the
 * check, which asks the executors and Linear afresh and takes up to a minute; the report of
 * a check before stays in view while a new one runs. A check that cannot run replaces the
 * page's content; the way back to the guide stays.
 */
export default function Doctor() {
  const { t } = useTranslation();
  const doctor = useDoctor();
  const { label } = useTriggerNames();
  // A new report is put on the page at low priority: the one on screen stays where it is until the
  // new one is ready to take its place, so that a click that has just been answered (a fix) is
  // not met by a page that changed under it in the middle of the click.
  const report = useDeferredValue(doctor.data);
  const header = (
    <PageHeader
      title={t("nav.doctor")}
      description={t("doctor.description")}
      actions={
        <>
          <Link to="/onboarding" className="rounded-sm text-sm underline underline-offset-3 outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50">
            {t("doctor.rerun_guide")}
          </Link>
          <Button variant="outline" size="sm" disabled={doctor.isFetching} onClick={() => void doctor.refetch()}>
            <RefreshCw aria-hidden className={cn(doctor.isFetching && "animate-spin")} />
            {t("doctor.recheck")}
          </Button>
        </>
      }
    />
  );

  if (doctor.isError && !doctor.isFetching) {
    return (
      <>
        {header}
        <ErrorState error={doctor.error} headline="doctor.failed" onRetry={() => void doctor.refetch()} />
      </>
    );
  }
  return (
    <>
      {header}
      <div className="flex flex-col gap-6">
        {doctor.isFetching && <Checking />}
        {report === undefined ? <LoadingState rows={4} /> : <Report report={report} labelName={label} />}
      </div>
    </>
  );
}

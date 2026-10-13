import { Unplug } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DetailLine } from "@/components/states";
import { WorkspaceCheck } from "@/components/WorkspaceCheck";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import type { DoctorReport } from "@/lib/types";
import { CheckedReport } from "./CheckedReport";
import { checkOf, type DoctorQuery, workspaceComplete } from "./check";
import { StepFrame } from "./StepFrame";
import type { StepNav } from "./steps";

/** Where the workspace could not be looked at, and why: no connection to Linear, or a check of it that failed. */
function WorkspaceUnchecked({ report }: { report: DoctorReport }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-3">
      <p className="flex items-center gap-2 rounded-xl border border-dashed bg-card p-4 text-muted-foreground">
        <Unplug aria-hidden className="size-4 shrink-0" />
        {t("doctor.workspace_unchecked")}
      </p>
      {report.linear_error !== null && (
        <div className="flex flex-col gap-1">
          <p className="font-medium text-destructive">{t("doctor.linear_check_failed")}</p>
          <DetailLine text={report.linear_error} />
        </div>
      )}
    </div>
  );
}

/**
 * The third step: the label and the statuses IssueRelay needs in the Linear workspace. What is
 * missing is explained (the program does not create it) and does not hold the guide: the label
 * and the statuses can be added any time, so the step that cannot say they are there is left with
 * "I'll add them later", and while the check runs the guide may go on.
 */
export function WorkspaceStep({ nav, doctor }: { nav: StepNav; doctor: DoctorQuery }) {
  const { t } = useTranslation();
  const { label } = useTriggerNames();
  const check = checkOf(doctor);
  const complete = check.phase === "loading" || (check.phase === "ready" && workspaceComplete(check.report));
  return (
    <StepFrame
      nav={nav}
      title={t("onboarding.title.workspace")}
      description={t("onboarding.description.workspace", { label })}
      forward={{ label: complete ? t("onboarding.next") : t("onboarding.later"), quiet: !complete }}
    >
      <CheckedReport doctor={doctor}>
        {(report) => (report.workspace === null ? <WorkspaceUnchecked report={report} /> : <WorkspaceCheck report={report.workspace} labelName={label} />)}
      </CheckedReport>
    </StepFrame>
  );
}

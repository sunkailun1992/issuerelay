import { ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { CheckRow } from "@/components/CheckRow";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useOpenLinear } from "@/hooks/useOpenLinear";
import type { WorkspaceReport } from "@/lib/types";

/** Linear's home page: all the program does about a label or a status that is missing is to point the way. */
const LINEAR_HOME = "https://linear.app/";

/**
 * The part of the environment check that looks at the Linear workspace (design 8, step 3): the
 * trigger label (`labelName`, from the settings) and the statuses IssueRelay moves an issue through,
 * in the teams that were looked at. What is missing is explained, with a button to Linear; the
 * program does not create it. Both the check and the way to add the label name it.
 */
export function WorkspaceCheck({ report, labelName }: { report: WorkspaceReport; labelName: string }) {
  const { t } = useTranslation();
  const open = useOpenLinear();
  const missingStates = report.missing_states.length > 0;
  return (
    <Card size="sm">
      <CardContent>
        <ul className="flex flex-col gap-1.5">
          <CheckRow ok={report.label_ok}>
            <span>{t(report.label_ok ? "doctor.label_ok" : "doctor.label_missing", { label: labelName })}</span>
          </CheckRow>
          <CheckRow ok={!missingStates}>
            <span>{missingStates ? t("doctor.states_missing", { states: report.missing_states.join(", ") }) : t("doctor.states_ok")}</span>
          </CheckRow>
        </ul>
        {report.teams_checked.length > 0 && (
          <p className="text-xs text-muted-foreground">{t("doctor.teams_checked", { teams: report.teams_checked.join(", ") })}</p>
        )}
        {(!report.label_ok || missingStates) && (
          <div className="flex flex-col items-start gap-2">
            {!report.label_ok && <p>{t("doctor.add_label_hint", { label: labelName })}</p>}
            {missingStates && <p>{t("doctor.add_states_hint")}</p>}
            <Button variant="outline" size="sm" onClick={() => open(LINEAR_HOME)}>
              <ExternalLink aria-hidden />
              {t("doctor.open_linear")}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

import { TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ExecutorCheck } from "@/components/ExecutorCheck";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { CheckedReport } from "./CheckedReport";
import { checkOf, type DoctorQuery, executorReady } from "./check";
import { StepFrame } from "./StepFrame";
import type { StepNav } from "./steps";

/**
 * The fourth step: Claude Code and Codex, each with what is missing and the way to mend it. The
 * guide goes on only when one of them is ready, since without one nothing can be run. A fix that
 * works makes the environment check outdated, and the check this step is showing runs again by
 * itself (`useSetupAction`), so the card is followed by the new state.
 */
export function ExecutorsStep({ nav, doctor }: { nav: StepNav; doctor: DoctorQuery }) {
  const { t } = useTranslation();
  const check = checkOf(doctor);
  const ready = check.phase === "ready" && check.report.executors.some(executorReady);
  return (
    <StepFrame
      nav={nav}
      title={t("onboarding.title.executors")}
      description={t("onboarding.description.executors")}
      forward={{ label: t("onboarding.next"), disabled: !ready }}
    >
      <CheckedReport doctor={doctor}>
        {(report) => (
          <>
            {!report.executors.some(executorReady) && (
              <Alert role="status">
                <TriangleAlert aria-hidden className="text-state-failed!" />
                <AlertTitle>{t("onboarding.no_executor")}</AlertTitle>
                <AlertDescription>{t("onboarding.no_executor_hint")}</AlertDescription>
              </Alert>
            )}
            {/* One column at the smallest window, two where there is room. */}
            <div className="grid grid-cols-[repeat(auto-fit,minmax(20rem,1fr))] gap-4">
              {report.executors.map((executor) => (
                <ExecutorCheck key={executor.kind} report={executor} />
              ))}
            </div>
          </>
        )}
      </CheckedReport>
    </StepFrame>
  );
}

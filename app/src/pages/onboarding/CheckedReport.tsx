import { LoaderCircle, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import type { DoctorReport } from "@/lib/types";
import { cn } from "@/lib/utils";
import { checkOf, type DoctorQuery } from "./check";

/** What the check is doing while it does it: it runs the executors and asks Linear, which takes up to a minute. */
function Checking() {
  const { t } = useTranslation();
  return (
    <p role="status" className="flex items-center gap-2 text-muted-foreground">
      <LoaderCircle aria-hidden className="size-4 shrink-0 animate-spin" />
      {t("doctor.checking")}
    </p>
  );
}

/**
 * The environment check as a step of the guide shows it: the report (`children` is given it) with
 * a button to check again, a placeholder and the time it takes while there is no report yet, and
 * the failure of a check that could not run, with a retry. A report stays in view while a new
 * check runs, with a note that it does.
 */
export function CheckedReport({ doctor, children }: { doctor: DoctorQuery; children: (report: DoctorReport) => ReactNode }) {
  const { t } = useTranslation();
  const check = checkOf(doctor);
  if (check.phase === "failed") {
    return <ErrorState error={check.error} headline="doctor.failed" onRetry={() => void doctor.refetch()} />;
  }
  if (check.phase === "loading") {
    return (
      <div className="flex flex-col gap-3">
        <Checking />
        <LoadingState rows={2} />
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-4">
      {check.checking && <Checking />}
      {children(check.report)}
      <div>
        <Button variant="outline" size="sm" disabled={doctor.isFetching} onClick={() => void doctor.refetch()}>
          <RefreshCw aria-hidden className={cn(doctor.isFetching && "animate-spin")} />
          {t("doctor.recheck")}
        </Button>
      </div>
    </div>
  );
}

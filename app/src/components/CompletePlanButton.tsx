import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { useCompletePlan } from "@/hooks/queries";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import type { Job, PlanProgress } from "@/lib/types";

/**
 * Closes a split plan whose child issues have all ended (plan-closure design). Like marking done it
 * changes Linear, so it asks first and says what it sets: the configured "done" status. The engine
 * checks the run, the plan issue and every child again after the click.
 */
export function CompletePlanButton({ job, progress }: { job: Job; progress: PlanProgress }) {
  const { t } = useTranslation();
  const completePlan = useCompletePlan();
  const { done } = useTriggerNames();
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button disabled={completePlan.isPending}>
          <Check aria-hidden />
          {t("plan.complete")}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("plan.complete")}</AlertDialogTitle>
          <AlertDialogDescription>
            {t("plan.complete_confirm", {
              identifier: job.issue_identifier,
              completed: progress.completed,
              canceled: progress.canceled,
              done,
            })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction onClick={() => completePlan.mutate(job.id)}>{t("actions.confirm")}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

import {
  CircleCheck,
  CircleSlash,
  CircleX,
  ClipboardCheck,
  Clock,
  LoaderCircle,
  MessageCircleQuestion,
  NotebookPen,
  Unplug,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { displayJobState } from "@/lib/runs";
import type { DisplayJobState, JobState } from "@/lib/types";
import type { VerificationStatus } from "@/lib/workflow-types";

/**
 * The colour and icon of each job state, the same everywhere a state is shown. The colours are
 * the `--state-*` variables of styles.css (text, a 30% border and a 10% tint); the class names
 * are written out in full so that Tailwind finds them.
 */
// eslint-disable-next-line react-refresh/only-export-components -- the table is shared with the tests; a change here reloads the page.
export const STATE_STYLE: Record<DisplayJobState, { icon: LucideIcon; className: string }> = {
  waiting_binding: { icon: Unplug, className: "text-state-waiting border-state-waiting/30 bg-state-waiting/10" },
  queued: { icon: Clock, className: "text-state-queued border-state-queued/30 bg-state-queued/10" },
  running: { icon: LoaderCircle, className: "text-state-running border-state-running/30 bg-state-running/10" },
  awaiting_acceptance: { icon: ClipboardCheck, className: "text-state-awaiting border-state-awaiting/30 bg-state-awaiting/10" },
  needs_info: { icon: MessageCircleQuestion, className: "text-state-needs-info border-state-needs-info/30 bg-state-needs-info/10" },
  done: { icon: CircleCheck, className: "text-state-done border-state-done/30 bg-state-done/10" },
  failed: { icon: CircleX, className: "text-state-failed border-state-failed/30 bg-state-failed/10" },
  canceled: { icon: CircleSlash, className: "text-state-canceled border-state-canceled/30 bg-state-canceled/10" },
};

/**
 * A planning run that posted its plan is `needs_info` for the engine: it waits for the user, in the
 * same colour, but with a notebook and its own name.
 */
// eslint-disable-next-line react-refresh/only-export-components -- shared with the tests like `STATE_STYLE`.
export const PLANNED_STYLE = { icon: NotebookPen, className: STATE_STYLE.needs_info.className };

/**
 * A posted plan whose child issues have all ended (plan-closure design): it waits for the user to
 * close it, in the colour of a run awaiting acceptance.
 */
// eslint-disable-next-line react-refresh/only-export-components -- shared with the tests like `STATE_STYLE`.
export const CLOSABLE_STYLE = { icon: ClipboardCheck, className: STATE_STYLE.awaiting_acceptance.className };

/**
 * A job's state as a coloured badge with its icon and its name in the interface language.
 * `planned`: the job is a planning run that posted its plan (`isPlanned`), shown as such.
 * `closable`: that plan's child issues have all ended, so it can be closed; only a posted plan uses it.
 */
export function StatusBadge({ state, planned = false, closable = false, verification }: {
  state: JobState;
  planned?: boolean;
  closable?: boolean;
  verification?: VerificationStatus;
}) {
  const { t } = useTranslation();
  const visibleState = verification === "passed" ? "done" : displayJobState(state);
  const asPlanned = planned && visibleState === "needs_info";
  const asClosable = asPlanned && closable;
  const { icon: Icon, className } = asClosable ? CLOSABLE_STYLE : asPlanned ? PLANNED_STYLE : STATE_STYLE[visibleState];
  const label = verification !== undefined ? t(`acceptance.verification_status.${verification}`)
    : asClosable ? t("state.plan_closable") : asPlanned ? t("state.planned") : t(`state.${visibleState}`);
  return (
    <Badge variant="outline" className={className}>
      <Icon aria-hidden className={state === "running" ? "animate-spin" : undefined} />
      {label}
    </Badge>
  );
}

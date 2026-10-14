import { ChevronRight, type LucideIcon } from "lucide-react";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Section } from "@/components/Section";
import { StatusBadge } from "@/components/StatusBadge";
import { JOB_STATES, type TriggerNames } from "@/lib/types";

/** The four conditions an issue has to meet (design 5.1, without the monitoring switch), in the order the page lists them. */
const CONDITIONS = ["assigned", "label", "todo", "bound"] as const;

/** The four conditions, worded with the label and the status the settings name. */
export function TriggerConditions({ names }: { names: TriggerNames }) {
  const { t } = useTranslation();
  return (
    <Section title={t("help.trigger_title")}>
      <p className="text-muted-foreground">{t("help.trigger_intro")}</p>
      <ol className="flex list-decimal flex-col gap-1.5 pl-6 marker:text-muted-foreground">
        {CONDITIONS.map((condition) => (
          <li key={condition}>{t(`help.conditions.${condition}`, { label: names.label, todo: names.todo })}</li>
        ))}
      </ol>
      <p className="text-muted-foreground">{t("help.trigger_note")}</p>
    </Section>
  );
}

/**
 * How an issue goes through Linear's statuses (the settings' names for them), with the one step
 * the user takes in the panel, and what each of the eight statuses the panel shows means.
 */
export function IssueFlow({ names }: { names: TriggerNames }) {
  const { t } = useTranslation();
  const steps = [names.todo, names.in_progress, names.in_testing, t("help.flow_mark_done"), names.done];
  return (
    <Section title={t("help.flow_title")}>
      <p>{t("help.flow_text", { todo: names.todo, in_progress: names.in_progress, in_testing: names.in_testing, done: names.done })}</p>
      <ol aria-label={t("help.flow_steps")} className="flex flex-wrap items-center gap-y-2">
        {steps.map((step, index) => (
          <li key={index} className="flex items-center">
            {index > 0 && <ChevronRight aria-hidden className="mx-1 size-4 shrink-0 text-muted-foreground" />}
            <span className="rounded-md border bg-card px-2.5 py-1 font-medium">{step}</span>
          </li>
        ))}
      </ol>
      <ul aria-label={t("help.states_title")} className="divide-y rounded-xl border bg-card">
        {JOB_STATES.map((state) => (
          <li key={state} className="flex items-start gap-3 px-4 py-3">
            {/* The room for the widest badge, so that every sentence starts at the same place. */}
            <div className="w-44 shrink-0">
              <StatusBadge state={state} />
            </div>
            <p className="min-w-0 flex-1 text-muted-foreground">{t(`help.state_meaning.${state}`)}</p>
          </li>
        ))}
      </ul>
    </Section>
  );
}

/** One example issue: what it says, and (for the vague one) what comes of it. */
export function ExampleIssue({ icon: Icon, label, title, body, note }: { icon: LucideIcon; label: string; title: string; body: string; note?: string }) {
  const labelId = useId();
  return (
    <div role="group" aria-labelledby={labelId} className="flex flex-col gap-2 rounded-xl border bg-card p-4">
      <p id={labelId} className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <Icon aria-hidden className="size-4 shrink-0" />
        {label}
      </p>
      <p className="font-medium">{title}</p>
      <p className="text-muted-foreground">{body}</p>
      {note !== undefined && <p className="text-xs text-muted-foreground">{note}</p>}
    </div>
  );
}

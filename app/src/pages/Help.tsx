import { ChevronRight, CircleCheck, MessageCircleQuestion } from "lucide-react";
import { type ReactNode, useId } from "react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/PageHeader";
import { Section } from "@/components/Section";
import { StatusBadge } from "@/components/StatusBadge";
import { useSettings } from "@/hooks/queries";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import { JOB_STATES, type TriggerNames } from "@/lib/types";

/** The four conditions an issue has to meet (design 5.1, without the monitoring switch), in the order the page lists them. */
const CONDITIONS = ["assigned", "label", "todo", "bound"] as const;

/** The four conditions, worded with the label and the status the settings name. */
function TriggerConditions({ names }: { names: TriggerNames }) {
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
function Flow({ names }: { names: TriggerNames }) {
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
function Example({ icon: Icon, label, title, body, note }: { icon: typeof CircleCheck; label: string; title: string; body: string; note?: string }) {
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

/** A clear issue and a vague one: what to write, and what the AI does with what it cannot work out. */
function Examples({ names }: { names: TriggerNames }) {
  const { t } = useTranslation();
  return (
    <Section title={t("help.example_title")}>
      <p className="text-muted-foreground">{t("help.example_intro")}</p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(16rem,1fr))] gap-4">
        <Example
          icon={CircleCheck}
          label={t("help.example.good_label")}
          title={t("help.example.good_title")}
          body={t("help.example.good_body")}
        />
        <Example
          icon={MessageCircleQuestion}
          label={t("help.example.vague_label")}
          title={t("help.example.vague_title")}
          body={t("help.example.vague_body")}
          note={t("help.example.vague_note", { todo: names.todo })}
        />
      </div>
    </Section>
  );
}

/** A question and what answers it. */
function Question({ question, children }: { question: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <h3 className="font-medium">{question}</h3>
      {children}
    </div>
  );
}

const PERMISSION_LEVELS = ["full", "auto", "personal"] as const;

function Questions({ names }: { names: TriggerNames }) {
  const { t } = useTranslation();
  return (
    <Section title={t("help.faq_title")}>
      <div className="flex flex-col gap-5">
        <Question question={t("help.faq.not_picked_q")}>
          <p className="text-muted-foreground">{t("help.faq.not_picked_a", { label: names.label, todo: names.todo })}</p>
        </Question>
        <Question question={t("help.faq.permission_q")}>
          <p className="text-muted-foreground">{t("help.faq.permission_a")}</p>
          <ul aria-label={t("help.faq.permission_list")} className="flex flex-col gap-1.5 text-muted-foreground">
            {PERMISSION_LEVELS.map((level) => (
              <li key={level}>
                <strong className="font-medium text-foreground">{t(`permission.${level}`)}</strong> {t(`permission.${level}_hint`)}
              </li>
            ))}
          </ul>
        </Question>
        <Question question={t("help.faq.proxy_q")}>
          <p className="text-muted-foreground">{t("help.faq.proxy_a")}</p>
        </Question>
        <Question question={t("help.faq.mcp_q")}>
          <p className="text-muted-foreground">{t("help.faq.mcp_a")}</p>
        </Question>
        <Question question={t("help.faq.logs_q")}>
          <p className="text-muted-foreground">{t("help.faq.logs_a")}</p>
        </Question>
      </div>
    </Section>
  );
}

/**
 * The help page (design 9.2): when IssueRelay takes an issue on, how an issue moves, how to write
 * one, and the questions that come up. It is all text from the translations, with the label and
 * the statuses of the settings put in; until the settings are read (and if they cannot be) the
 * default names stand, and in that case the page says so under its title.
 */
export default function Help() {
  const { t } = useTranslation();
  const settings = useSettings();
  const names = useTriggerNames();
  return (
    <>
      <PageHeader title={t("nav.help")} description={t("help.description")} />
      {settings.isError && !settings.isFetching && <p className="mb-4 text-muted-foreground">{t("help.defaults_note")}</p>}
      <div className="flex flex-col gap-8">
        <TriggerConditions names={names} />
        <Flow names={names} />
        <Examples names={names} />
        <Questions names={names} />
      </div>
    </>
  );
}

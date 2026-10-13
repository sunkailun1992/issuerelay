import { CircleCheck, MessageCircleQuestion } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ExampleIssue, IssueFlow, TriggerConditions } from "@/components/HelpSections";
import { PageHeader } from "@/components/PageHeader";
import { Section } from "@/components/Section";
import { useSettings } from "@/hooks/queries";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import type { TriggerNames } from "@/lib/types";

/** A clear issue and a vague one: what to write, and what the AI does with what it cannot work out. */
function Examples({ names }: { names: TriggerNames }) {
  const { t } = useTranslation();
  return (
    <Section title={t("help.example_title")}>
      <p className="text-muted-foreground">{t("help.example_intro")}</p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(16rem,1fr))] gap-4">
        <ExampleIssue
          icon={CircleCheck}
          label={t("help.example.good_label")}
          title={t("help.example.good_title")}
          body={t("help.example.good_body")}
        />
        <ExampleIssue
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
        <IssueFlow names={names} />
        <Examples names={names} />
        <Questions names={names} />
      </div>
    </>
  );
}

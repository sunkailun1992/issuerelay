import type { UseQueryResult } from "@tanstack/react-query";
import { CircleCheck, Info } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ExampleIssue, IssueFlow, TriggerConditions } from "@/components/HelpSections";
import { Section } from "@/components/Section";
import { ErrorState } from "@/components/states";
import { Alert, AlertTitle } from "@/components/ui/alert";
import { useBindings } from "@/hooks/queries";
import { useTriggerNames } from "@/hooks/useTriggerNames";
import type { Settings } from "@/lib/types";
import { StepFrame } from "./StepFrame";
import type { StepNav } from "./steps";

/**
 * The last step: what an issue needs before IssueRelay takes it on (the four conditions, one
 * example), how it moves from there, and how Claude Code and Codex can be given the control
 * server. It is the same text as the help page. The button saves the settings with the guide
 * completed (`finish`, which the guide's page owns, since it also goes to the overview) and, with
 * settings that cannot be read, waits for them to be: the saved settings are a whole copy of them.
 */
export function FinishStep({
  nav,
  settings,
  finish,
  finishing,
}: {
  nav: StepNav;
  settings: UseQueryResult<Settings, Error>;
  finish: () => void;
  finishing: boolean;
}) {
  const { t } = useTranslation();
  const names = useTriggerNames();
  const bindings = useBindings();
  const nothingBound = bindings.data !== undefined && bindings.data.length === 0;
  return (
    <StepFrame
      nav={nav}
      title={t("onboarding.title.finish")}
      description={t("onboarding.description.finish")}
      forward={{ label: t("onboarding.start"), disabled: settings.data === undefined, pending: finishing, onClick: finish }}
    >
      {settings.isError && !settings.isFetching && settings.data === undefined && (
        <ErrorState error={settings.error} headline="settings.load_failed" onRetry={() => void settings.refetch()} />
      )}
      {nothingBound && (
        <Alert role="status">
          <Info aria-hidden />
          <AlertTitle>{t("onboarding.no_binding_note")}</AlertTitle>
        </Alert>
      )}
      <TriggerConditions names={names} />
      <Section title={t("help.example_title")}>
        <p className="text-muted-foreground">{t("help.example_intro")}</p>
        <ExampleIssue
          icon={CircleCheck}
          label={t("help.example.good_label")}
          title={t("help.example.good_title")}
          body={t("help.example.good_body")}
        />
      </Section>
      <IssueFlow names={names} />
      <p className="text-muted-foreground">{t("onboarding.control_server_note")}</p>
    </StepFrame>
  );
}

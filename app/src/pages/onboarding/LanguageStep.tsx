import { useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useId } from "react";
import { useTranslation } from "react-i18next";
import { ErrorState, LoadingState } from "@/components/states";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { queryKeys, useAppInfo, useSaveSettings, useSettings } from "@/hooks/queries";
import { LANGUAGES, resolveLanguage, setLanguage } from "@/i18n";
import { reportFailure } from "@/lib/errors";
import type { Settings } from "@/lib/types";
import { StepFrame } from "./StepFrame";
import type { StepNav } from "./steps";

/** What can be chosen: the system's language, then the languages of the panel. */
const CHOICES = ["system", ...LANGUAGES] as const;

/**
 * The first step: the language of the panel. A choice is followed at once and then saved.
 * The panel follows the language the settings say (`LanguageSync` in App), so the settings in the
 * cache are changed together with the language, or the app would set the old language back until
 * the save has been read again. A save that fails puts both back, and only then says so, so that
 * the failure is told in the language the panel is in again.
 */
export function LanguageStep({ nav }: { nav: StepNav }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const settings = useSettings();
  const info = useAppInfo();
  const save = useSaveSettings({ quiet: true });
  const base = useId();
  const languageOf = (setting: string) => resolveLanguage(setting, info.data?.system_locale ?? undefined);

  const choose = async (value: string) => {
    const before = settings.data;
    if (before === undefined || value === before.language) return;
    const chosen: Settings = { ...before, language: value };
    client.setQueryData(queryKeys.settings, chosen);
    void setLanguage(languageOf(value));
    try {
      await save.mutateAsync(chosen);
    } catch (error) {
      // Not when another choice has followed meanwhile: that one stands.
      if (client.getQueryData<Settings>(queryKeys.settings)?.language === value) {
        client.setQueryData(queryKeys.settings, before);
        void setLanguage(languageOf(before.language));
      }
      reportFailure(error, t, "actions.failed.save_settings");
    }
  };

  let content: ReactNode;
  if (settings.isError && !settings.isFetching && settings.data === undefined) {
    content = <ErrorState error={settings.error} headline="settings.load_failed" onRetry={() => void settings.refetch()} />;
  } else if (settings.data === undefined) {
    content = <LoadingState rows={3} />;
  } else {
    content = (
      <RadioGroup value={settings.data.language} onValueChange={(value) => void choose(value)} aria-label={t("settings.language")} className="gap-2">
        {CHOICES.map((choice) => {
          const id = `${base}-${choice}`;
          return (
            <label
              key={choice}
              htmlFor={id}
              className="flex cursor-pointer items-center gap-3 rounded-lg border bg-card p-3 text-sm font-medium has-[[data-state=checked]]:border-primary has-[[data-state=checked]]:bg-primary/5 has-focus-visible:ring-3 has-focus-visible:ring-ring/50"
            >
              <RadioGroupItem id={id} value={choice} />
              {/* A language is named in its own language, which a screen reader needs to be told. */}
              <span lang={choice === "system" ? undefined : choice}>{choice === "system" ? t("settings.follow_system") : t(`languages.${choice}`)}</span>
            </label>
          );
        })}
      </RadioGroup>
    );
  }

  return (
    <StepFrame nav={nav} title={t("onboarding.title.language")} description={t("onboarding.description.language")} forward={{ label: t("onboarding.next") }}>
      {content}
    </StepFrame>
  );
}

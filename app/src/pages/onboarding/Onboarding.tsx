import { X } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { Button } from "@/components/ui/button";
import { DOCTOR_FRESH_MS, useDoctor, useSaveSettings, useSettings, useStatus } from "@/hooks/queries";
import { BindingStep } from "./BindingStep";
import { ExecutorsStep } from "./ExecutorsStep";
import { FinishStep } from "./FinishStep";
import { LanguageStep } from "./LanguageStep";
import { LinearStep } from "./LinearStep";
import { Stepper } from "./Stepper";
import { STEPS, type StepNav } from "./steps";
import { WorkspaceStep } from "./WorkspaceStep";

/**
 * The first-run guide (design 8), outside the shell: the steps down the left, the step with its
 * Back and forward buttons on the right, and in the header the way out, which a step that holds
 * the guide back (Linear not connected, no executor ready, a check that keeps failing) would
 * otherwise leave without: no sidebar to leave by. A first run leaves with "Set up later", which
 * saves `onboarding_completed` like the last step does and goes to the overview (on the last step
 * its own button is that way out); a re-run (the guide is completed already) leaves with "Exit the
 * guide", which goes there and saves nothing.
 * The first-run gate sends a first run here, and the environment check leads back here at any
 * time; it always starts at the first step and each step shows how things stand now. The saves are
 * read again before they are over, so the gate does not send the user back.
 */
export default function Onboarding() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [index, setIndex] = useState(0);
  // The furthest step the user has been to: what the list ticks and lets the user go back to.
  const [reached, setReached] = useState(0);
  const step = STEPS[index];
  // Read at the start, so that the steps that need them (the connection, the end) open on their data and not on a placeholder.
  useStatus();
  const settings = useSettings();
  // One environment check serves the workspace and the executors, which follow each other: it
  // runs when the first of them is entered, is not repeated between the two (nor when they are
  // entered again soon after: going back and forth must not run it each time, it takes up to a
  // minute), and is made again when a fix, a binding or the connection makes it outdated.
  const doctor = useDoctor(step === "workspace" || step === "executors", { staleTime: DOCTOR_FRESH_MS });
  const save = useSaveSettings();

  /** Ends the guide: the settings are saved with it completed, and the overview follows. */
  const finish = () => {
    if (settings.data === undefined) return;
    save.mutate({ ...settings.data, onboarding_completed: true }, { onSuccess: () => navigate("/", { replace: true }) });
  };
  const goTo = (to: number) => {
    setIndex(to);
    setReached((furthest) => Math.max(furthest, to));
  };
  const nav: StepNav = {
    index,
    locked: save.isPending,
    onBack: () => setIndex((at) => Math.max(at - 1, 0)),
    onNext: () => goTo(Math.min(index + 1, STEPS.length - 1)),
  };

  // Which way out depends on whether the guide has been completed before: until that is known
  // there is none to offer, and when it cannot be read the way to the overview is the one left
  // (its gate says why the settings are not there). On a first run's last step "Start using"
  // saves what "Set up later" would, so the header does not offer it twice.
  let leave: ReactNode = null;
  if (settings.data !== undefined && !settings.data.onboarding_completed) {
    if (step !== "finish") {
      leave = (
        <Button variant="ghost" size="sm" disabled={save.isPending} onClick={finish}>
          <X aria-hidden />
          {t("onboarding.set_up_later")}
        </Button>
      );
    }
  } else if (settings.data !== undefined || settings.isError) {
    leave = (
      <Button variant="ghost" size="sm" disabled={save.isPending} onClick={() => navigate("/", { replace: true })}>
        <X aria-hidden />
        {t("onboarding.exit")}
      </Button>
    );
  }

  let view: ReactNode;
  switch (step) {
    case "language":
      view = <LanguageStep nav={nav} />;
      break;
    case "linear":
      view = <LinearStep nav={nav} />;
      break;
    case "workspace":
      view = <WorkspaceStep nav={nav} doctor={doctor} />;
      break;
    case "executors":
      view = <ExecutorsStep nav={nav} doctor={doctor} />;
      break;
    case "binding":
      view = <BindingStep nav={nav} />;
      break;
    case "finish":
      view = <FinishStep nav={nav} settings={settings} finish={finish} finishing={save.isPending} />;
      break;
  }

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex shrink-0 items-center gap-2 border-b px-(--page-padding) py-3">
        <img src="/icon.svg" alt="" aria-hidden className="size-8 shrink-0" />
        <div className="flex min-w-0 flex-1 items-baseline gap-3">
          <span className="shrink-0 text-base font-semibold">{t("app.name")}</span>
          <span className="truncate text-muted-foreground">{t("app.tagline")}</span>
        </div>
        {leave && <div className="shrink-0">{leave}</div>}
      </header>
      <div className="flex min-h-0 flex-1">
        <Stepper index={index} reached={reached} locked={nav.locked} onGo={setIndex} />
        <main className="flex min-w-0 flex-1 flex-col">{view}</main>
      </div>
    </div>
  );
}

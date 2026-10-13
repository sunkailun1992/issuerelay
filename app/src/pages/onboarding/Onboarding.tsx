import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { useDoctor, useSaveSettings, useSettings, useStatus } from "@/hooks/queries";
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
 * Back and forward buttons on the right. The first-run gate sends a first run here, and the
 * environment check leads back here at any time; it always starts at the first step and each
 * step shows how things stand now. The last step saves `onboarding_completed` and goes to the
 * overview; the save is read again before it is over, so the gate does not send the user back.
 */
export default function Onboarding() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [index, setIndex] = useState(0);
  const step = STEPS[index];
  // Read at the start, so that the steps that need them (the connection, the end) open on their data and not on a placeholder.
  useStatus();
  const settings = useSettings();
  // One environment check serves the workspace and the executors, which follow each other: it
  // runs when the first of them is entered, is not repeated between the two, and is made again
  // when they are entered afresh or a fix makes it outdated.
  const doctor = useDoctor(step === "workspace" || step === "executors");
  const save = useSaveSettings();

  const finish = () => {
    if (settings.data === undefined) return;
    save.mutate({ ...settings.data, onboarding_completed: true }, { onSuccess: () => navigate("/", { replace: true }) });
  };
  const nav: StepNav = {
    index,
    locked: save.isPending,
    onBack: () => setIndex((at) => Math.max(at - 1, 0)),
    onNext: () => setIndex((at) => Math.min(at + 1, STEPS.length - 1)),
  };

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
        <span className="text-base font-semibold">{t("app.name")}</span>
      </header>
      <div className="flex min-h-0 flex-1">
        <Stepper index={index} locked={nav.locked} onGo={setIndex} />
        <main className="flex min-w-0 flex-1 flex-col">{view}</main>
      </div>
    </div>
  );
}

import { ArrowLeft, LoaderCircle } from "lucide-react";
import { type ReactNode, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { STEPS, type StepNav } from "./steps";

/** The button that leaves the step forward. */
export interface Forward {
  label: string;
  disabled?: boolean;
  /** The step is saving: the button waits, with a spinner. */
  pending?: boolean;
  /** The step is left unfinished (to be done later): a quieter button than the one that goes on. */
  quiet?: boolean;
  /** Goes on to the next step unless the step does something else (the last one ends the guide). */
  onClick?: () => void;
}

/**
 * What every step of the guide is made of: its number, its heading and the sentence under it, its
 * content, and the Back and forward buttons at the bottom. The content scrolls inside the frame
 * and the buttons stay where they are, so that at the smallest window they are always in reach.
 * The heading is given the focus when the step opens: the button that was pressed to get here goes
 * with the step before, and a screen reader would otherwise say nothing about the new one.
 */
export function StepFrame({
  nav,
  title,
  description,
  forward,
  children,
}: {
  nav: StepNav;
  title: string;
  description?: ReactNode;
  forward: Forward;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
  }, []);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-(--content-max-width) p-(--page-padding)">
          <p className="mb-1 text-xs font-medium text-muted-foreground">
            {t("onboarding.step_of", { current: nav.index + 1, total: STEPS.length })}
          </p>
          <PageHeader title={title} description={description} headingRef={heading} />
          <div className="flex flex-col gap-6">{children}</div>
        </div>
      </div>
      <footer className="shrink-0 border-t bg-background">
        <div className="mx-auto flex w-full max-w-(--content-max-width) items-center justify-between gap-3 px-(--page-padding) py-3">
          <Button variant="outline" disabled={nav.index === 0 || nav.locked} onClick={nav.onBack}>
            <ArrowLeft aria-hidden />
            {t("onboarding.back")}
          </Button>
          <Button
            variant={forward.quiet ? "outline" : "default"}
            disabled={forward.disabled || forward.pending || nav.locked}
            onClick={forward.onClick ?? nav.onNext}
          >
            {forward.pending && <LoaderCircle aria-hidden className="animate-spin" />}
            {forward.label}
          </Button>
        </div>
      </footer>
    </div>
  );
}

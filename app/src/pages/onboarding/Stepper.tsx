import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { STEPS } from "./steps";

/** One row of the list: the number (or the tick) and the name. */
const ROW = "flex w-full items-center gap-3 rounded-md px-3 py-2 text-left outline-none";

/**
 * The steps of the guide, down the left: the one the user is on is highlighted (and is the
 * `aria-current` step), the steps behind are ticked and can be gone back to, the steps ahead are
 * text only. A step that is ahead is never an entry: it is reached with the forward button, which
 * a step can hold back until what it needs is there. `locked` holds the steps behind too.
 */
export function Stepper({ index, locked, onGo }: { index: number; locked: boolean; onGo: (index: number) => void }) {
  const { t } = useTranslation();
  return (
    <nav aria-label={t("onboarding.steps_label")} className="w-(--sidebar-width) shrink-0 overflow-y-auto border-r border-sidebar-border bg-sidebar p-2 text-sidebar-foreground">
      <ol className="flex flex-col gap-1">
        {STEPS.map((step, at) => {
          const name = t(`onboarding.step.${step}`);
          const done = at < index;
          const current = at === index;
          const mark = (
            <span
              aria-hidden
              className={cn(
                "flex size-6 shrink-0 items-center justify-center rounded-full border text-xs font-medium",
                current && "border-transparent bg-primary text-primary-foreground",
                done && "border-transparent bg-state-done/10 text-state-done",
                !current && !done && "border-border text-muted-foreground",
              )}
            >
              {done ? <Check className="size-3.5" /> : at + 1}
            </span>
          );
          return (
            <li key={step} aria-current={current ? "step" : undefined}>
              {done ? (
                <button
                  type="button"
                  aria-label={t("onboarding.step_done", { step: name })}
                  disabled={locked}
                  onClick={() => onGo(at)}
                  className={cn(ROW, "text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground focus-visible:ring-3 focus-visible:ring-sidebar-ring/50 disabled:pointer-events-none disabled:opacity-50")}
                >
                  {mark}
                  {name}
                </button>
              ) : (
                <div className={cn(ROW, current ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground" : "text-muted-foreground")}>
                  {mark}
                  {name}
                </div>
              )}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}

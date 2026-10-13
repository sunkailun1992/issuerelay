import { CircleCheck, Plus } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { MiddleEllipsis } from "@/components/MiddleEllipsis";
import { ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { useBindings } from "@/hooks/queries";
import { cleanDisplayText } from "@/lib/clean";
import { BindingForm } from "@/pages/BindingDialog";
import { StepFrame } from "./StepFrame";
import type { StepNav } from "./steps";

/** Where the focus is put after the view changes: the news of a saved binding, the heading of the form for one more, the button that asks for it. */
type FocusTarget = "saved" | "adding" | "add";

/**
 * The fifth step: the first binding of a Linear project to a folder, with the form of the
 * bindings page (and its risk warning for full access). With no binding yet the form is shown;
 * once there is one (made here, or there from before) the bound projects are listed, with a way
 * to add another, and the guide goes on with Next; the step can also be left with "Bind later".
 * The form is gone when a binding is saved, and the focus would fall to the page with it, so it
 * is put on the news of the save, on the heading of the form for another binding, and back on
 * the button that asked for that form.
 */
export function BindingStep({ nav }: { nav: StepNav }) {
  const { t } = useTranslation();
  const bindings = useBindings();
  const listId = useId();
  const addingId = useId();
  // A form for one more binding is open (it is a new, empty one each time: the list takes its place in between).
  const [adding, setAdding] = useState(false);
  // A binding was saved here: the list says so until another form is asked for.
  const [saved, setSaved] = useState(false);
  const [focus, setFocus] = useState<{ target: FocusTarget } | null>(null);
  const news = useRef<HTMLParagraphElement>(null);
  const addingHeading = useRef<HTMLHeadingElement>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (focus === null) return;
    const element = { saved: news, adding: addingHeading, add: addButton }[focus.target].current;
    element?.focus();
  }, [focus]);

  const list = bindings.data;
  const hasBinding = list !== undefined && list.length > 0;
  const showForm = adding || (list !== undefined && list.length === 0 && !saved);

  let content: ReactNode;
  if (bindings.isError && !bindings.isFetching) {
    content = <ErrorState error={bindings.error} headline="bindings.load_failed" onRetry={() => void bindings.refetch()} />;
  } else if (list === undefined) {
    content = <LoadingState rows={2} />;
  } else if (showForm) {
    content = (
      <section aria-labelledby={adding ? addingId : undefined} className="flex flex-col gap-4">
        {adding && (
          <h2 id={addingId} ref={addingHeading} tabIndex={-1} className="text-base font-semibold outline-none">
            {t("onboarding.add_another_title")}
          </h2>
        )}
        <BindingForm
          onSaved={() => {
            setAdding(false);
            setSaved(true);
            setFocus({ target: "saved" });
          }}
          onCancel={
            adding
              ? () => {
                  setAdding(false);
                  setFocus({ target: "add" });
                }
              : undefined
          }
        />
      </section>
    );
  } else {
    content = (
      <div className="flex flex-col gap-4">
        {saved && (
          <p role="status" ref={news} tabIndex={-1} className="flex items-center gap-2 font-medium outline-none">
            <CircleCheck aria-hidden className="size-4 shrink-0 text-state-done" />
            {t("onboarding.binding_saved")}
          </p>
        )}
        <div className="flex flex-col gap-2">
          <h2 id={listId} className="text-base font-semibold">
            {t("onboarding.bound_title")}
          </h2>
          <ul aria-labelledby={listId} className="flex flex-col gap-2">
            {list.map((binding) => (
              <li key={binding.id} className="flex flex-col gap-0.5 rounded-xl border bg-card px-4 py-3">
                <span className="font-medium wrap-anywhere">{cleanDisplayText(binding.project_name)}</span>
                <MiddleEllipsis text={cleanDisplayText(binding.directory)} className="font-mono text-xs text-muted-foreground" />
              </li>
            ))}
          </ul>
        </div>
        <div>
          <Button
            variant="outline"
            ref={addButton}
            onClick={() => {
              setAdding(true);
              setSaved(false);
              setFocus({ target: "adding" });
            }}
          >
            <Plus aria-hidden />
            {t("onboarding.add_another")}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <StepFrame
      nav={nav}
      title={t("onboarding.title.binding")}
      description={t("onboarding.description.binding")}
      forward={{ label: hasBinding ? t("onboarding.next") : t("onboarding.bind_later"), quiet: !hasBinding }}
    >
      {content}
    </StepFrame>
  );
}

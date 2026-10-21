import { useQueryClient } from "@tanstack/react-query";
import { ChevronDown, CircleCheck, CircleX, FolderOpen, LoaderCircle, Pencil, Plus, Trash2 } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useId, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { MiddleEllipsis } from "@/components/MiddleEllipsis";
import { PageHeader } from "@/components/PageHeader";
import { DetailLine, EmptyState, ErrorState, LoadingState } from "@/components/states";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { queryKeys, useBindings, useDoctor, useRemoveBinding, useSettings } from "@/hooks/queries";
import { type BindingLabels, bindingLabels, executorVerdict } from "@/lib/binding-labels";
import { cleanDisplayLines, cleanDisplayText } from "@/lib/clean";
import { describeError } from "@/lib/errors";
import type { Binding, BindingReport, DoctorReport } from "@/lib/types";
import { cn } from "@/lib/utils";
import { BindingDialog } from "./BindingDialog";

/** The environment check as the cards show it. */
interface LastCheck {
  /** The report of the last check, while nothing it looked at has changed since. */
  report: DoctorReport | undefined;
  /** A check is running. */
  checking: boolean;
  /** The last check failed, and nothing has been checked since. */
  error: unknown;
  run: () => void;
}

/**
 * The last environment check. The check runs the executors and asks Linear, so a visit to this
 * page does not run it: a report is there when the environment check page or the guide ran one
 * (it is the same cached report), or when a card's button runs it. A change that decides what
 * the check looks at (a binding saved or removed, the settings, the Linear connection) marks the
 * report outdated, and an outdated report is not shown as if it still held.
 */
function useLastCheck(): LastCheck {
  const client = useQueryClient();
  const doctor = useDoctor(false);
  const subscribe = useCallback((notify: () => void) => client.getQueryCache().subscribe(notify), [client]);
  const outdated = useSyncExternalStore(subscribe, () => client.getQueryState(queryKeys.doctor)?.isInvalidated === true);
  return {
    report: outdated ? undefined : doctor.data,
    checking: doctor.isFetching,
    error: doctor.isError ? doctor.error : null,
    run: () => void doctor.refetch(),
  };
}

/** One line of a check: a symbol and a sentence (the symbol is never the only way to tell). */
function Verdict({ ok, children }: { ok: boolean; children: ReactNode }) {
  const Icon = ok ? CircleCheck : CircleX;
  return (
    <li className="flex items-center gap-1.5">
      <Icon aria-hidden className={cn("size-4 shrink-0", ok ? "text-state-done" : "text-state-failed")} />
      <span>{children}</span>
    </li>
  );
}

/**
 * What the last check found about a binding: its folder and its executor. Before any check there
 * is a button for one; an executor that is not ready and a folder that is missing are mended in the
 * environment check, which is one click away. `describedBy` names the element that says which
 * project this is, as for the card's other buttons.
 */
function CheckResult({ check, report, describedBy }: { check: LastCheck; report: BindingReport | undefined; describedBy: string }) {
  const { t } = useTranslation();
  const again = (
    <Button variant="outline" size="sm" aria-describedby={describedBy} onClick={check.run}>
      {t("bindings.recheck")}
    </Button>
  );
  if (check.checking) {
    return (
      <span className="flex items-center gap-1.5 text-muted-foreground">
        <LoaderCircle aria-hidden className="size-4 animate-spin" />
        {t("bindings.checking")}
      </span>
    );
  }
  if (check.error !== null) {
    const failure = describeError(check.error, t, "bindings.check_failed");
    return (
      <div className="flex flex-col items-start gap-1.5">
        <span className="font-medium text-destructive">{failure.headline}</span>
        {failure.detail !== null && <DetailLine text={failure.detail} />}
        {again}
      </div>
    );
  }
  if (report === undefined) {
    return (
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-muted-foreground">{t("bindings.not_checked")}</span>
        {again}
      </div>
    );
  }
  return (
    <div className="flex flex-col items-start gap-1.5">
      <ul className="flex flex-col gap-1">
        <Verdict ok={report.directory_exists}>{report.directory_exists ? t("bindings.directory_ok") : t("bindings.directory_missing")}</Verdict>
        <Verdict ok={report.executor_ready}>{executorVerdict(t, report)}</Verdict>
      </ul>
      {(!report.directory_exists || !report.executor_ready) && (
        <Link to="/doctor" className="rounded-sm text-primary underline-offset-4 outline-none hover:underline focus-visible:ring-3 focus-visible:ring-ring/50">
          {t("bindings.open_doctor")}
        </Link>
      )}
    </div>
  );
}

/**
 * Asks before a binding goes: the issues of its project wait for a new one. `describedBy` names
 * the element that says which project this is, so that the button's description tells it.
 * `onRemoving` and `onKept` tell the page that a removal was confirmed and that it failed: once the
 * card is gone, its button is gone with the focus Radix gave back to it.
 */
function RemoveButton({
  binding,
  describedBy,
  onRemoving,
  onKept,
}: {
  binding: Binding;
  describedBy: string;
  onRemoving: () => void;
  onKept: () => void;
}) {
  const { t } = useTranslation();
  const remove = useRemoveBinding();
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button variant="outline" size="sm" disabled={remove.isPending} aria-describedby={describedBy}>
          <Trash2 aria-hidden />
          {t("bindings.remove")}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("bindings.remove_title")}</AlertDialogTitle>
          <AlertDialogDescription>{t("bindings.remove_confirm", { project: binding.project_name })}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => {
              onRemoving();
              remove.mutate(binding.id, { onError: onKept });
            }}
          >
            {t("actions.confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** The prompt override of a binding: "Yes", which opens to the text, or "None". */
function PromptOverride({ text, labelId }: { text: string | undefined; labelId: string }) {
  const { t } = useTranslation();
  const toggleId = useId();
  if (text === undefined || text.trim() === "") return <span className="text-muted-foreground">{t("bindings.prompt_none")}</span>;
  return (
    <Collapsible>
      <CollapsibleTrigger asChild>
        {/* Read with the name of the row ("Prompt override Yes"), since "Yes" alone says nothing. */}
        <Button id={toggleId} variant="ghost" size="xs" aria-labelledby={`${labelId} ${toggleId}`} className="group/prompt -ml-2">
          {t("bindings.prompt_yes")}
          <ChevronDown aria-hidden className="transition-transform group-data-[state=open]/prompt:rotate-180" />
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <pre className="mt-2 max-h-48 overflow-auto rounded-md bg-muted p-2 font-mono text-xs break-words whitespace-pre-wrap">
          {cleanDisplayLines(text)}
        </pre>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** What a card tells the page: its Edit was pressed (with the button), a removal was confirmed, or it failed. */
interface CardActions {
  onEdit: (opener: HTMLElement) => void;
  onRemoving: () => void;
  onKept: () => void;
}

/**
 * One binding (design 9.2): its folder, executor, permission level, whether its runs are lean, model,
 * reasoning effort, prompt override and what the last check found.
 * Edit, Remove and Check again are named for what they do and described by the project, which their
 * name leaves out (they are one in every card).
 */
function BindingCard({ binding, labels, check, actions }: { binding: Binding; labels: BindingLabels; check: LastCheck; actions: CardActions }) {
  const { t } = useTranslation();
  const titleId = useId();
  const promptId = useId();
  const report = check.report?.bindings.find((entry) => entry.binding_id === binding.id);
  const directory = cleanDisplayText(binding.directory);
  return (
    <section aria-labelledby={titleId}>
      <Card size="sm">
        <div className="flex flex-wrap items-center justify-between gap-2 px-(--card-spacing)">
          <h2 id={titleId} className="min-w-0 text-base font-medium break-words">
            {binding.project_name}
          </h2>
          <div className="flex shrink-0 items-center gap-2">
            <Button variant="outline" size="sm" aria-describedby={titleId} onClick={(event) => actions.onEdit(event.currentTarget)}>
              <Pencil aria-hidden />
              {t("bindings.edit")}
            </Button>
            <RemoveButton binding={binding} describedBy={titleId} onRemoving={actions.onRemoving} onKept={actions.onKept} />
          </div>
        </div>
        <CardContent>
          <dl className="grid grid-cols-[max-content_minmax(0,1fr)] items-baseline gap-x-4 gap-y-2">
            <dt className="text-muted-foreground">{t("bindings.directory")}</dt>
            <dd className="min-w-0">
              <MiddleEllipsis text={directory} className="font-mono text-xs" />
            </dd>
            <dt className="text-muted-foreground">{t("bindings.executor")}</dt>
            <dd>{labels.executor(binding.executor)}</dd>
            <dt className="text-muted-foreground">{t("bindings.permission")}</dt>
            <dd>{labels.permission(binding.permission)}</dd>
            <dt className="text-muted-foreground">{t("bindings.lean")}</dt>
            <dd>{labels.lean(binding, report?.lean)}</dd>
            <dt className="text-muted-foreground">{t("bindings.model")}</dt>
            <dd className={cn(binding.model === undefined ? "text-muted-foreground" : "font-mono text-xs")}>
              {binding.model ?? t("bindings.model_default")}
            </dd>
            <dt className="text-muted-foreground">{t("bindings.effort")}</dt>
            <dd>{labels.effort(binding.effort)}</dd>
            <dt id={promptId} className="text-muted-foreground">
              {t("bindings.prompt")}
            </dt>
            <dd className="min-w-0">
              <PromptOverride text={binding.prompt_override} labelId={promptId} />
            </dd>
            <dt className="text-muted-foreground">{t("bindings.check")}</dt>
            <dd aria-live="polite" className="min-w-0">
              <CheckResult check={check} report={report} describedBy={titleId} />
            </dd>
          </dl>
        </CardContent>
      </Card>
    </section>
  );
}

/**
 * The project bindings (design 9.2): every binding with its folder, executor, permission level,
 * model, prompt override and what the last check found, and the way to add, edit and remove one.
 * The bindings come from the configuration, so reading them can fail on its own.
 */
export default function Bindings() {
  const { t } = useTranslation();
  const bindings = useBindings();
  const settings = useSettings();
  const check = useLastCheck();
  const labels = bindingLabels(t, settings.data);
  // The binding being edited stays while the dialog fades out, so that it does not turn into a new one.
  const [dialog, setDialog] = useState<{ open: boolean; binding?: Binding }>({ open: false });
  // The button that opened the dialog, taken from its click: in WebKit a mouse click does not
  // focus a button, so the dialog would not know it from the focus it had when it opened.
  const opener = useRef<HTMLElement | null>(null);
  const open = (binding: Binding | undefined, from: HTMLElement) => {
    opener.current = from;
    setDialog({ open: true, binding });
  };
  // Where the focus goes when what had it is gone (the button of the empty page goes once there is
  // a binding, a card goes with its binding): the header's button, else the heading.
  const addButton = useRef<HTMLButtonElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const pageFocus = useCallback(() => addButton.current ?? heading.current, []);
  const fallbackFocus = useCallback(() => (opener.current?.isConnected ? opener.current : null) ?? pageFocus(), [pageFocus]);
  // A confirmed removal takes the card away with its Remove button, which had the focus: once the
  // bindings are read without it, and the focus fell to the document, it goes to the page's focus.
  const removing = useRef(false);
  useEffect(() => {
    if (!removing.current) return;
    removing.current = false;
    if (document.activeElement === null || document.activeElement === document.body) pageFocus()?.focus();
  }, [bindings.data, pageFocus]);
  const actionsOf = (binding: Binding): CardActions => ({
    onEdit: (from) => open(binding, from),
    onRemoving: () => {
      removing.current = true;
    },
    onKept: () => {
      removing.current = false;
    },
  });

  let actions: ReactNode;
  let body: ReactNode;
  if (bindings.isError && !bindings.isFetching) {
    body = <ErrorState error={bindings.error} headline="bindings.load_failed" onRetry={() => void bindings.refetch()} />;
  } else if (bindings.data === undefined || (settings.data === undefined && !settings.isError)) {
    // The settings say what "Default" stands for in every card.
    body = <LoadingState rows={3} />;
  } else if (bindings.data.length === 0) {
    body = (
      <EmptyState
        icon={FolderOpen}
        title={t("bindings.empty")}
        description={t("bindings.empty_description")}
        action={
          <Button onClick={(event) => open(undefined, event.currentTarget)}>
            <Plus aria-hidden />
            {t("bindings.add_first")}
          </Button>
        }
      />
    );
  } else {
    actions = (
      <Button size="sm" ref={addButton} onClick={(event) => open(undefined, event.currentTarget)}>
        <Plus aria-hidden />
        {t("bindings.add")}
      </Button>
    );
    body = (
      <div className="flex flex-col gap-4">
        {bindings.data.map((binding) => (
          <BindingCard key={binding.id} binding={binding} labels={labels} check={check} actions={actionsOf(binding)} />
        ))}
      </div>
    );
  }

  return (
    <>
      <PageHeader title={t("nav.bindings")} description={t("bindings.description")} actions={actions} headingRef={heading} />
      {body}
      <BindingDialog
        open={dialog.open}
        initial={dialog.binding}
        onOpenChange={(open) => setDialog((current) => ({ ...current, open }))}
        fallbackFocus={fallbackFocus}
      />
    </>
  );
}

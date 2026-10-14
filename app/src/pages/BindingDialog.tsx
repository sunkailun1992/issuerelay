import { CircleAlert, FolderOpen, LoaderCircle } from "lucide-react";
import { type FormEvent, type ReactNode, useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Field, Hint } from "@/components/FormField";
import { FullAccessWarning } from "@/components/FullAccessWarning";
import { ErrorState, LinearConnecting, LoadingState } from "@/components/states";
import { Alert, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { useBindings, useProjects, useSaveBinding, useSaveSettings, useSettings, useStatus } from "@/hooks/queries";
import { useFocusReturn } from "@/hooks/useFocusReturn";
import { useBackend } from "@/lib/backend-context";
import { bindingLabels } from "@/lib/binding-labels";
import { reportFailure } from "@/lib/errors";
import { isValidModelName } from "@/lib/model-name";
import type { Binding, ExecutorKind, PermissionProfile, Settings } from "@/lib/types";

/** The entry of the executor and permission lists that names none: the binding follows the settings. */
const DEFAULT = "default";

interface FieldsProps {
  initial?: Binding;
  settings: Settings;
  onSaved: () => void;
  onCancel?: () => void;
  /** Told when the form starts and when it stops saving, so that a dialog around it can stay open meanwhile. */
  onBusyChange?: (busy: boolean) => void;
}

/**
 * The fields of a binding (design 8 step 5, 9.2). The Linear project and the folder are needed;
 * the executor, the permission level, the model and the prompt override are left to the settings
 * and the executor unless named. Whatever the engine refuses is told at the top of the form, in its
 * own words under a translated headline. The first time full access would take effect, the user
 * is asked to understand what it means (design 14) and the answer is kept in the settings.
 */
function BindingFields({ initial, settings, onSaved, onCancel, onBusyChange }: FieldsProps) {
  const { t } = useTranslation();
  const backend = useBackend();
  const labels = bindingLabels(t, settings);
  const base = useId();
  const id = (name: string) => `${base}-${name}`;
  const creating = initial === undefined;
  // Only a new binding picks its project; Linear is not asked for the projects otherwise.
  const projects = useProjects(creating);
  const connecting = useStatus().data?.linear.kind === "connecting";
  const bindings = useBindings();
  // The form shows a failed save itself, so the hook does not also toast it.
  const save = useSaveBinding({ quiet: true });
  const saveSettings = useSaveSettings();
  const saveButton = useRef<HTMLButtonElement>(null);
  // Set while the confirmation of the warning is being handled, from the moment it is given:
  // `busy` follows a draw later, and two presses can come before that.
  const confirming = useRef(false);

  const [projectId, setProjectId] = useState("");
  const [directory, setDirectory] = useState(initial?.directory ?? "");
  const [executor, setExecutor] = useState<ExecutorKind | typeof DEFAULT>(initial?.executor ?? DEFAULT);
  const [permission, setPermission] = useState<PermissionProfile | typeof DEFAULT>(initial?.permission ?? DEFAULT);
  const [model, setModel] = useState(initial?.model ?? "");
  const [prompt, setPrompt] = useState(initial?.prompt_override ?? "");
  const [warning, setWarning] = useState(false);
  // The project that was chosen is not among the projects any more by the time it is saved.
  const [missing, setMissing] = useState(false);

  // A blank around a pasted name is not part of it.
  const modelName = model.trim();
  const modelInvalid = modelName !== "" && !isValidModelName(modelName);
  const effectivePermission = permission === DEFAULT ? settings.default_permission : permission;
  const needsWarning = effectivePermission === "full" && !settings.full_access_acknowledged;
  const busy = save.isPending || saveSettings.isPending;
  const canSave = (creating ? projectId !== "" : true) && directory !== "" && !modelInvalid && !busy;
  // A project that has a binding already cannot be bound again: saving would replace that binding.
  const bound = new Set(bindings.data?.map((binding) => binding.project_id));

  // A failure of the save is only shown here: a dialog around the form must not close while it is awaited.
  useEffect(() => {
    onBusyChange?.(busy);
    return () => onBusyChange?.(false);
  }, [busy, onBusyChange]);

  /** Whatever is changed, a failure told before is about what was there before. */
  const edited = () => {
    if (save.isError) save.reset();
    if (missing) setMissing(false);
  };

  const choose = async () => {
    try {
      const chosen = await backend.chooseDirectory();
      // Closing the picker without a choice leaves the folder as it was.
      if (chosen !== null) {
        edited();
        setDirectory(chosen);
      }
    } catch (error) {
      reportFailure(error, t, "bindings.choose_failed");
    }
  };

  /**
   * What stands for the project: its name when a binding is edited (the project of a binding does
   * not change), else the list of Linear's projects, or why there is none yet (the first connection
   * to Linear still under way is a wait, not a failure). `control` is whether it is something the
   * label can name.
   */
  const projectField = (): { control: boolean; node: ReactNode } => {
    if (initial !== undefined) {
      // Dimmed, so that it does not look like a field to fill in (`!`: the input's own background is not merged).
      return {
        control: true,
        node: <Input id={id("project")} readOnly value={initial.project_name} className="bg-muted! text-muted-foreground" />,
      };
    }
    if (connecting) return { control: false, node: <LinearConnecting /> };
    if (projects.isError && !projects.isFetching) {
      return {
        control: false,
        node: <ErrorState error={projects.error} headline="bindings.projects_failed" onRetry={() => void projects.refetch()} />,
      };
    }
    if (bindings.isError && !bindings.isFetching && bindings.data === undefined) {
      return {
        control: false,
        node: <ErrorState error={bindings.error} headline="bindings.load_failed" onRetry={() => void bindings.refetch()} />,
      };
    }
    // Which projects are taken is known once the bindings are read.
    if (projects.data === undefined || bindings.data === undefined) {
      return { control: false, node: <Skeleton role="status" aria-label={t("common.loading")} className="h-9 w-full" /> };
    }
    if (projects.data.length === 0) {
      return { control: false, node: <p className="text-muted-foreground">{t("bindings.no_projects")}</p> };
    }
    return {
      control: true,
      node: (
        <Select
          value={projectId}
          onValueChange={(value) => {
            edited();
            setProjectId(value);
          }}
        >
          <SelectTrigger id={id("project")} className="w-full">
            <SelectValue placeholder={t("bindings.project_placeholder")} />
          </SelectTrigger>
          <SelectContent>
            {projects.data.map((project) => (
              <SelectItem key={project.id} value={project.id} disabled={bound.has(project.id)}>
                {bound.has(project.id) ? t("bindings.project_bound", { name: project.name }) : project.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ),
    };
  };

  /** The binding as the form has it, or null while there is no project to bind. */
  const build = (): Binding | null => {
    let named: Pick<Binding, "id" | "project_id" | "project_name">;
    if (initial !== undefined) {
      named = { id: initial.id, project_id: initial.project_id, project_name: initial.project_name };
    } else {
      const project = projects.data?.find((entry) => entry.id === projectId);
      if (project === undefined) return null;
      named = { id: project.id, project_id: project.id, project_name: project.name };
    }
    const binding: Binding = { ...named, directory };
    if (executor !== DEFAULT) binding.executor = executor;
    if (permission !== DEFAULT) binding.permission = permission;
    if (modelName !== "") binding.model = modelName;
    // Typed text is kept as typed; only an override of blanks counts as none.
    if (prompt.trim() !== "") binding.prompt_override = prompt;
    return binding;
  };

  const persist = async () => {
    const binding = build();
    if (binding === null) {
      // The list of projects was read again meanwhile, without the one that was chosen: say so, and
      // let the list show that nothing is chosen.
      setProjectId("");
      setMissing(true);
      return;
    }
    try {
      await save.mutateAsync(binding);
    } catch {
      // Shown at the top of the form (`save.error`).
      return;
    }
    onSaved();
  };

  const understood = async () => {
    // The warning stays for the length of its fade, and its button can be pressed again meanwhile.
    if (confirming.current) return;
    confirming.current = true;
    try {
      setWarning(false);
      try {
        await saveSettings.mutateAsync({ ...settings, full_access_acknowledged: true });
      } catch {
        // The settings could not take the confirmation (the hook has told the user): nothing is bound
        // on the strength of it.
        return;
      }
      await persist();
    } finally {
      confirming.current = false;
    }
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSave) return;
    if (needsWarning) setWarning(true);
    else void persist();
  };

  const project = projectField();
  return (
    <>
      <form onSubmit={submit} noValidate className="flex min-h-0 flex-col gap-4">
        {save.isError && <ErrorState error={save.error} headline="actions.failed.save_binding" />}
        {missing && (
          <Alert variant="destructive">
            <CircleAlert aria-hidden />
            <AlertTitle>{t("bindings.project_missing")}</AlertTitle>
          </Alert>
        )}
        {/* Scrolls when the window is low; the padding keeps the focus rings from being cut off. */}
        <div className="-m-1 flex min-h-0 flex-col gap-4 overflow-y-auto p-1">
          <Field label={t("bindings.project")} htmlFor={project.control ? id("project") : undefined}>
            {project.node}
          </Field>

          <Field label={t("bindings.directory")} htmlFor={id("directory")}>
            <div className="flex gap-2">
              {/* The folder is chosen with the picker, not typed; the engine says whether it will do. */}
              <Input
                id={id("directory")}
                readOnly
                value={directory}
                title={directory || undefined}
                placeholder={t("bindings.directory_placeholder")}
                className="min-w-0 font-mono"
              />
              {/* An existing binding starts here: the project above it cannot be changed, this can. */}
              <Button type="button" variant="outline" autoFocus={!creating} onClick={() => void choose()}>
                <FolderOpen aria-hidden />
                {t("bindings.choose")}
              </Button>
            </div>
          </Field>

          <Field label={t("bindings.executor")} htmlFor={id("executor")}>
            <Select
              value={executor}
              onValueChange={(value) => {
                edited();
                setExecutor(value as ExecutorKind | typeof DEFAULT);
              }}
            >
              <SelectTrigger id={id("executor")} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={DEFAULT}>{labels.executor(undefined)}</SelectItem>
                <SelectItem value="claude">{labels.executor("claude")}</SelectItem>
                <SelectItem value="codex">{labels.executor("codex")}</SelectItem>
              </SelectContent>
            </Select>
          </Field>

          <Field label={t("bindings.permission")} htmlFor={id("permission")}>
            <Select
              value={permission}
              onValueChange={(value) => {
                edited();
                setPermission(value as PermissionProfile | typeof DEFAULT);
              }}
            >
              <SelectTrigger id={id("permission")} className="w-full" aria-describedby={id("permission-hint")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={DEFAULT}>{labels.permission(undefined)}</SelectItem>
                <SelectItem value="full">{labels.permission("full")}</SelectItem>
                <SelectItem value="auto">{labels.permission("auto")}</SelectItem>
                <SelectItem value="personal">{labels.permission("personal")}</SelectItem>
              </SelectContent>
            </Select>
            <Hint id={id("permission-hint")}>{t(`permission.${effectivePermission}_hint`)}</Hint>
          </Field>

          <Field label={t("bindings.model")} htmlFor={id("model")}>
            <Input
              id={id("model")}
              value={model}
              onChange={(event) => {
                edited();
                setModel(event.target.value);
              }}
              aria-invalid={modelInvalid}
              aria-describedby={id("model-hint")}
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              className="font-mono"
            />
            {modelInvalid ? (
              <Hint id={id("model-hint")} tone="error">
                {t("bindings.model_invalid")}
              </Hint>
            ) : (
              <Hint id={id("model-hint")}>{t("bindings.model_hint")}</Hint>
            )}
          </Field>

          <Field label={t("bindings.prompt")} htmlFor={id("prompt")}>
            <Textarea
              id={id("prompt")}
              value={prompt}
              onChange={(event) => {
                edited();
                setPrompt(event.target.value);
              }}
              aria-describedby={id("prompt-hint")}
              rows={4}
              className="max-h-40"
            />
            <Hint id={id("prompt-hint")}>{t("bindings.prompt_hint")}</Hint>
          </Field>
        </div>

        <div className="flex justify-end gap-2">
          {onCancel && (
            <Button type="button" variant="outline" disabled={busy} onClick={onCancel}>
              {t("common.cancel")}
            </Button>
          )}
          <Button type="submit" ref={saveButton} disabled={!canSave}>
            {busy && <LoaderCircle aria-hidden className="animate-spin" />}
            {t("common.save")}
          </Button>
        </div>
      </form>
      {/* Where the user was gets the focus back; the Save button when that is gone. */}
      <FullAccessWarning open={warning} onConfirm={() => void understood()} onCancel={() => setWarning(false)} returnFocusTo={saveButton} />
    </>
  );
}

/**
 * The body of the binding dialog, which the first-run guide uses as well: a new binding, or the
 * one given as `initial`. `onSaved` is called once the binding is saved and the list of bindings
 * read again; `onCancel` adds a cancel button for a dialog, also while the settings are being read
 * or cannot be. The settings are needed for what "Default" stands for and for the full-access
 * warning, so the form waits for them.
 */
export function BindingForm(props: Omit<FieldsProps, "settings">) {
  const { t } = useTranslation();
  const settings = useSettings();
  if (settings.data === undefined) {
    return (
      <div className="flex flex-col gap-4">
        {settings.isError && !settings.isFetching ? (
          <ErrorState error={settings.error} headline="bindings.settings_failed" onRetry={() => void settings.refetch()} />
        ) : (
          <LoadingState rows={4} />
        )}
        {props.onCancel && (
          <div className="flex justify-end">
            <Button type="button" variant="outline" onClick={props.onCancel}>
              {t("common.cancel")}
            </Button>
          </div>
        )}
      </div>
    );
  }
  return <BindingFields {...props} settings={settings.data} />;
}

/**
 * A new binding, or `initial` edited, in a dialog. It is as high as the window allows and the
 * fields scroll inside it; the library's own close button has a fixed English label, so it is left
 * out: Cancel and Escape close it. It does not close while a save is on its way: a failure of the
 * save is shown in the form, and a closed form would lose it. When it closes the focus goes back
 * to what opened it (the page opens it, there is no trigger to do that), or to `fallbackFocus`
 * when that is gone.
 */
export function BindingDialog({
  open,
  onOpenChange,
  initial,
  fallbackFocus,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initial?: Binding;
  fallbackFocus?: () => HTMLElement | null;
}) {
  const { t } = useTranslation();
  const returnFocus = useFocusReturn(open, fallbackFocus);
  const saving = useRef(false);
  const savingChanged = useCallback((busy: boolean) => {
    saving.current = busy;
  }, []);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        // Escape and a click outside ask to close it; not while a save is on its way.
        if (!next && saving.current) return;
        onOpenChange(next);
      }}
    >
      {/* The library's grid, width and gap are replaced with `!`: its classes are not merged with these. */}
      <DialogContent
        showCloseButton={false}
        onCloseAutoFocus={returnFocus}
        className="flex! max-h-[calc(100vh-2rem)] flex-col gap-4! sm:max-w-lg!"
      >
        <DialogHeader>
          <DialogTitle>{initial === undefined ? t("bindings.title_add") : t("bindings.title_edit")}</DialogTitle>
          <DialogDescription>{t("bindings.dialog_description")}</DialogDescription>
        </DialogHeader>
        <BindingForm initial={initial} onSaved={() => onOpenChange(false)} onCancel={() => onOpenChange(false)} onBusyChange={savingChanged} />
      </DialogContent>
    </Dialog>
  );
}

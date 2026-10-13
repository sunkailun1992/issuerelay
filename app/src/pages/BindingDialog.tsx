import { FolderOpen, LoaderCircle } from "lucide-react";
import { type FormEvent, type ReactNode, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { FullAccessWarning } from "@/components/FullAccessWarning";
import { ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { useBindings, useProjects, useSaveBinding, useSaveSettings, useSettings } from "@/hooks/queries";
import { useBackend } from "@/lib/backend-context";
import { bindingLabels } from "@/lib/binding-labels";
import { reportFailure } from "@/lib/errors";
import { isValidModelName } from "@/lib/model-name";
import type { Binding, ExecutorKind, PermissionProfile, Settings } from "@/lib/types";
import { cn } from "@/lib/utils";

/** The entry of the executor and permission lists that names none: the binding follows the settings. */
const DEFAULT = "default";

/** A labelled field: the label, then the control and what is said about it. */
function Field({ label, htmlFor, children }: { label: string; htmlFor: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
    </div>
  );
}

/** A line under a control. */
function Hint({ id, tone = "muted", children }: { id: string; tone?: "muted" | "error"; children: ReactNode }) {
  return (
    <p id={id} className={cn("text-xs", tone === "error" ? "text-destructive" : "text-muted-foreground")}>
      {children}
    </p>
  );
}

interface FieldsProps {
  initial?: Binding;
  settings: Settings;
  onSaved: () => void;
  onCancel?: () => void;
}

/**
 * The fields of a binding (design 8 step 5, 9.2). The Linear project and the folder are needed;
 * the executor, the permission level, the model and the prompt override are left to the settings
 * and the executor unless named. Whatever the engine refuses is told at the top of the form, in its
 * own words under a translated headline. The first time full access would take effect, the user
 * is asked to understand what it means (design 14) and the answer is kept in the settings.
 */
function BindingFields({ initial, settings, onSaved, onCancel }: FieldsProps) {
  const { t } = useTranslation();
  const backend = useBackend();
  const labels = bindingLabels(t, settings);
  const base = useId();
  const id = (name: string) => `${base}-${name}`;
  const creating = initial === undefined;
  // Only a new binding picks its project; Linear is not asked for the projects otherwise.
  const projects = useProjects(creating);
  const bindings = useBindings();
  // The form shows a failed save itself, so the hook does not also toast it.
  const save = useSaveBinding({ quiet: true });
  const saveSettings = useSaveSettings();

  const [projectId, setProjectId] = useState("");
  const [directory, setDirectory] = useState(initial?.directory ?? "");
  const [executor, setExecutor] = useState<ExecutorKind | typeof DEFAULT>(initial?.executor ?? DEFAULT);
  const [permission, setPermission] = useState<PermissionProfile | typeof DEFAULT>(initial?.permission ?? DEFAULT);
  const [model, setModel] = useState(initial?.model ?? "");
  const [prompt, setPrompt] = useState(initial?.prompt_override ?? "");
  const [warning, setWarning] = useState(false);

  // A blank around a pasted name is not part of it.
  const modelName = model.trim();
  const modelInvalid = modelName !== "" && !isValidModelName(modelName);
  const effectivePermission = permission === DEFAULT ? settings.default_permission : permission;
  const needsWarning = effectivePermission === "full" && !settings.full_access_acknowledged;
  const busy = save.isPending || saveSettings.isPending;
  const canSave = (creating ? projectId !== "" : true) && directory !== "" && !modelInvalid && !busy;
  // A project that has a binding already cannot be bound again: saving would replace that binding.
  const bound = new Set(bindings.data?.map((binding) => binding.project_id));

  /** Whatever is changed, a failure told before is about what was there before. */
  const edited = () => {
    if (save.isError) save.reset();
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
   * not change), else the list of Linear's projects, or why there is none yet.
   */
  const projectControl = (): ReactNode => {
    if (initial !== undefined) {
      // Dimmed, so that it does not look like a field to fill in (`!`: the input's own background is not merged).
      return <Input id={id("project")} readOnly value={initial.project_name} className="bg-muted! text-muted-foreground" />;
    }
    if (projects.isError && !projects.isFetching) {
      return <ErrorState error={projects.error} headline="bindings.projects_failed" onRetry={() => void projects.refetch()} />;
    }
    if (bindings.isError && !bindings.isFetching && bindings.data === undefined) {
      return <ErrorState error={bindings.error} headline="bindings.load_failed" onRetry={() => void bindings.refetch()} />;
    }
    // Which projects are taken is known once the bindings are read.
    if (projects.data === undefined || bindings.data === undefined) {
      return <Skeleton role="status" aria-label={t("common.loading")} className="h-9 w-full" />;
    }
    if (projects.data.length === 0) return <p className="text-muted-foreground">{t("bindings.no_projects")}</p>;
    return (
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
              {project.name}
              {bound.has(project.id) && <span className="text-muted-foreground"> ({t("bindings.project_bound")})</span>}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
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
    if (binding === null) return;
    try {
      await save.mutateAsync(binding);
    } catch {
      // Shown at the top of the form (`save.error`).
      return;
    }
    onSaved();
  };

  const understood = async () => {
    setWarning(false);
    try {
      await saveSettings.mutateAsync({ ...settings, full_access_acknowledged: true });
    } catch {
      // The settings could not take the confirmation (the hook has told the user): nothing is bound
      // on the strength of it.
      return;
    }
    await persist();
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSave) return;
    if (needsWarning) setWarning(true);
    else void persist();
  };

  return (
    <>
      <form onSubmit={submit} noValidate className="flex min-h-0 flex-col gap-4">
        {save.isError && <ErrorState error={save.error} headline="actions.failed.save_binding" />}
        {/* Scrolls when the window is low; the padding keeps the focus rings from being cut off. */}
        <div className="-m-1 flex min-h-0 flex-col gap-4 overflow-y-auto p-1">
          <Field label={t("bindings.project")} htmlFor={id("project")}>
            {projectControl()}
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
              <Button type="button" variant="outline" onClick={() => void choose()}>
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
            <Button type="button" variant="outline" onClick={onCancel}>
              {t("common.cancel")}
            </Button>
          )}
          <Button type="submit" disabled={!canSave}>
            {busy && <LoaderCircle aria-hidden className="animate-spin" />}
            {t("common.save")}
          </Button>
        </div>
      </form>
      <FullAccessWarning open={warning} onConfirm={() => void understood()} onCancel={() => setWarning(false)} />
    </>
  );
}

/**
 * The body of the binding dialog, which the first-run guide uses as well: a new binding, or the
 * one given as `initial`. `onSaved` is called once the binding is saved and the list of bindings
 * read again; `onCancel` adds a cancel button for a dialog. The settings are needed for what
 * "Default" stands for and for the full-access warning, so the form waits for them.
 */
export function BindingForm(props: Omit<FieldsProps, "settings">) {
  const settings = useSettings();
  if (settings.data === undefined) {
    return settings.isError && !settings.isFetching ? (
      <ErrorState error={settings.error} headline="bindings.settings_failed" onRetry={() => void settings.refetch()} />
    ) : (
      <LoadingState rows={4} />
    );
  }
  return <BindingFields {...props} settings={settings.data} />;
}

/**
 * A new binding, or `initial` edited, in a dialog. It is as high as the window allows and the
 * fields scroll inside it; the library's own close button has a fixed English label, so it is left
 * out: Cancel and Escape close the dialog.
 */
export function BindingDialog({ open, onOpenChange, initial }: { open: boolean; onOpenChange: (open: boolean) => void; initial?: Binding }) {
  const { t } = useTranslation();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {/* The library's grid, width and gap are replaced with `!`: its classes are not merged with these. */}
      <DialogContent showCloseButton={false} className="flex! max-h-[calc(100vh-2rem)] flex-col gap-4! sm:max-w-lg!">
        <DialogHeader>
          <DialogTitle>{initial === undefined ? t("bindings.title_add") : t("bindings.title_edit")}</DialogTitle>
          <DialogDescription>{t("bindings.dialog_description")}</DialogDescription>
        </DialogHeader>
        <BindingForm initial={initial} onSaved={() => onOpenChange(false)} onCancel={() => onOpenChange(false)} />
      </DialogContent>
    </Dialog>
  );
}

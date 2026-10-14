import { LoaderCircle, Plus, Trash2 } from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ApiKeyForm } from "@/components/ApiKeyForm";
import { Field, Hint } from "@/components/FormField";
import { FullAccessWarning } from "@/components/FullAccessWarning";
import { PageHeader } from "@/components/PageHeader";
import { ErrorState, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { WithCommands } from "@/components/WithCommands";
import {
  useAppInfo,
  useAutostart,
  useCliPath,
  useControlServers,
  useInstallCliOnPath,
  useRemoveCliFromPath,
  useSaveSettings,
  useSetAutostart,
  useSetupAction,
  useSettings,
} from "@/hooks/queries";
import { useCopyText } from "@/hooks/useCopyText";
import { LANGUAGES } from "@/i18n/language";
import { cleanDisplayText } from "@/lib/clean";
import {
  type CountField,
  type Draft,
  MINIMUM,
  type Problem,
  problemsOf,
  sameDraft,
  draftOf,
  settingsOf,
  unsaved,
} from "@/lib/settings-form";
import type {
  CliPathStatus,
  ControlServer,
  ExecutorKind,
  NotificationSettings,
  PermissionProfile,
  Settings as SettingsData,
  TriggerNames,
} from "@/lib/types";
import { cn } from "@/lib/utils";

/** The command that builds IssueRelay's command-line program, for a development build that has none yet. */
const BUILD_CLI = "cargo build -p issuerelay-cli";
/** The command the PATH switch makes available in a terminal. */
const CLI_COMMAND = "issuerelay";

/** A group of settings: a card named by its heading. */
function Group({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={id}>
      <Card size="sm">
        <div className="px-(--card-spacing)">
          <h2 id={id} className="text-base font-medium">
            {title}
          </h2>
        </div>
        <CardContent className="gap-4">{children}</CardContent>
      </Card>
    </section>
  );
}

/** A heading inside a group that has parts. */
function Part({ title }: { title: string }) {
  return <h3 className="text-sm font-medium text-muted-foreground">{title}</h3>;
}

/** A row with a switch: its name on the left, the switch on the right, and a line of explanation under it. */
function SwitchRow({
  id,
  label,
  note,
  describedBy,
  checked,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  note?: ReactNode;
  /** The id of more text outside the row's own note that describes the switch, such as the state it is in. */
  describedBy?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  const described = [note === undefined ? undefined : `${id}-note`, describedBy].filter((part) => part !== undefined).join(" ");
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={id}>{label}</Label>
        <Switch id={id} checked={checked} disabled={disabled} aria-describedby={described === "" ? undefined : described} onCheckedChange={onChange} />
      </div>
      {note !== undefined && <Hint id={`${id}-note`}>{note}</Hint>}
    </div>
  );
}

/** What is wrong with a field, in words. */
function problemText(t: ReturnType<typeof useTranslation>["t"], problem: Problem): string {
  switch (problem.kind) {
    case "min":
      return t("settings.min_value", { min: problem.min });
    case "whole":
      return t("settings.whole_number");
    case "name":
      return t("settings.name_invalid");
  }
}

/** A text or number field with the problem of its value under it. */
function TextField({
  id,
  label,
  value,
  problem,
  type = "text",
  min,
  mono,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  problem: Problem | undefined;
  type?: "text" | "number";
  min?: number;
  mono?: boolean;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <Field label={label} htmlFor={id}>
      <Input
        id={id}
        type={type}
        inputMode={type === "number" ? "numeric" : undefined}
        min={min}
        step={type === "number" ? 1 : undefined}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        aria-invalid={problem !== undefined}
        aria-describedby={problem === undefined ? undefined : `${id}-problem`}
        autoComplete="off"
        spellCheck={false}
        className={cn(mono && "font-mono")}
      />
      {problem !== undefined && (
        <Hint id={`${id}-problem`} tone="error">
          {problemText(t, problem)}
        </Hint>
      )}
    </Field>
  );
}

/**
 * A list of languages: the entry that stands for no choice ("follow the system", "follow the
 * interface"), then the five the panel has, each by its own name. A value that is none of them (a
 * file written by hand, or by an older version) is listed as it is, so that the field says what is
 * saved and saving it changes nothing.
 */
function LanguageField({
  id,
  label,
  value,
  first,
  hint,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  first: { value: string; label: string };
  hint?: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  const known = value === first.value || (LANGUAGES as string[]).includes(value);
  return (
    <Field label={label} htmlFor={id}>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger id={id} className="w-full" aria-describedby={hint === undefined ? undefined : `${id}-hint`}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={first.value}>{first.label}</SelectItem>
          {LANGUAGES.map((lang) => (
            <SelectItem key={lang} value={lang}>
              {t(`languages.${lang}`)}
            </SelectItem>
          ))}
          {!known && value !== "" && <SelectItem value={value}>{cleanDisplayText(value)}</SelectItem>}
        </SelectContent>
      </Select>
      {hint !== undefined && <Hint id={`${id}-hint`}>{hint}</Hint>}
    </Field>
  );
}

/** Starting at login: the system's own setting, so it takes effect at once and is not saved with the page. */
function Autostart() {
  const { t } = useTranslation();
  const id = useId();
  const autostart = useAutostart();
  const change = useSetAutostart();
  if (autostart.isError && !autostart.isFetching) {
    return <ErrorState error={autostart.error} headline="settings.autostart_failed" onRetry={() => void autostart.refetch()} />;
  }
  if (autostart.data === undefined) return <LoadingState rows={1} />;
  // As asked while the system is told, and until what the system says is read again; a change it
  // refused is not shown, only told (by the action).
  const asked = change.isPending || (change.isSuccess && autostart.isFetching) ? change.variables : undefined;
  return (
    <SwitchRow
      id={id}
      label={t("settings.autostart")}
      note={t("settings.applies_at_once")}
      checked={asked ?? autostart.data}
      disabled={change.isPending}
      onChange={(enabled) => change.mutate(enabled)}
    />
  );
}

/**
 * What the PATH row says under its switch. The paths are the system's text: cleaned, and allowed to
 * break anywhere. The sentence about the state has the `id` the switch is described by, which is
 * how the reason a switch cannot be used is read with it; the shell line and its button are not
 * part of that description. Nothing is said while it is off.
 */
function CliPathNote({ id, status }: { id: string; status: CliPathStatus }) {
  const { t } = useTranslation();
  const copy = useCopyText();
  switch (status.kind) {
    case "off":
      return null;
    case "on": {
      const line = status.shell_line;
      if (line === null) {
        return (
          <p id={id} className="text-xs text-muted-foreground">
            {t("settings.cli.on_new_terminal")}
          </p>
        );
      }
      return (
        <div className="flex flex-col gap-1">
          <p id={id} className="text-xs text-muted-foreground">
            {t("settings.cli.shell_line")}
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <code className="min-w-0 rounded bg-muted px-2 py-1 font-mono text-xs wrap-anywhere text-foreground select-text">{line}</code>
            <Button size="sm" variant="outline" onClick={() => copy(line)}>
              {t("common.copy")}
            </Button>
          </div>
        </div>
      );
    }
    case "stale":
      return (
        <p id={id} className="text-xs text-muted-foreground wrap-anywhere">
          {t("settings.cli.stale", { target: cleanDisplayText(status.target) })}
        </p>
      );
    case "blocked":
      return (
        <p id={id} className="text-xs text-destructive wrap-anywhere">
          {t("settings.cli.blocked", { path: cleanDisplayText(status.path) })}
        </p>
      );
    case "unavailable":
      return (
        <p id={id} className="text-xs text-muted-foreground">
          {t(`settings.cli.unavailable.${status.reason}`)}
        </p>
      );
  }
}

/**
 * Putting the command line on PATH (design 12). It acts at once and is not saved with the page,
 * like starting at login; the switch shows only once the state is read, so it is never shown in a
 * state it is not in. Only where the app runs from does not allow it is the switch locked. A file
 * IssueRelay did not make in the way leaves it usable, shown off: the user removes or renames the
 * file, then turns it on, and the command looks at the slot again (it links if the file is gone,
 * and says "in the way" again if not, which is an answer and no failure).
 */
function CliOnPath() {
  const { t } = useTranslation();
  const id = useId();
  const status = useCliPath();
  const install = useInstallCliOnPath();
  const remove = useRemoveCliFromPath();
  if (status.isError && !status.isFetching) {
    return <ErrorState error={status.error} headline="settings.cli.load_failed" onRetry={() => void status.refetch()} />;
  }
  if (status.data === undefined) return <LoadingState rows={1} />;
  const current = status.data;
  const stateId = `${id}-state`;
  return (
    <div className="flex flex-col gap-1">
      <SwitchRow
        id={id}
        label={t("settings.cli.title")}
        note={<WithCommands text={t("settings.cli.description", { command: CLI_COMMAND })} commands={[CLI_COMMAND]} />}
        describedBy={current.kind === "off" ? undefined : stateId}
        checked={current.kind === "on"}
        disabled={install.isPending || remove.isPending || status.isFetching || current.kind === "unavailable"}
        onChange={(on) => (on ? install.mutate() : remove.mutate())}
      />
      <CliPathNote id={stateId} status={current} />
    </div>
  );
}

/** One executor's row: whether IssueRelay's control server is registered with it, and the button that changes that. */
function ControlServerRow({
  executor,
  state,
  cliMissing,
  checking,
  onCheckAgain,
}: {
  executor: ExecutorKind;
  state: ControlServer;
  cliMissing: boolean;
  checking: boolean;
  onCheckAgain: () => void;
}) {
  const { t } = useTranslation();
  const labelId = useId();
  const setup = useSetupAction();
  const name = t(`executor.${executor}`);
  const spinner = setup.isPending && <LoaderCircle aria-hidden className="animate-spin" />;
  return (
    <li aria-labelledby={labelId} className="flex flex-wrap items-center justify-between gap-2">
      <span id={labelId} className="font-medium">
        {name}
      </span>
      <div className="flex flex-wrap items-center gap-3">
        <span className="text-muted-foreground">{t(`settings.control_${state}`)}</span>
        {state === "not_registered" && (
          <Button
            variant="outline"
            size="sm"
            disabled={cliMissing || setup.isPending}
            onClick={() => setup.mutate({ kind: "register_control_server", executor })}
          >
            {spinner || <Plus aria-hidden />}
            {t("settings.register", { executor: name })}
          </Button>
        )}
        {state === "registered" && (
          <Button variant="outline" size="sm" disabled={setup.isPending} onClick={() => setup.mutate({ kind: "remove_control_server", executor })}>
            {spinner || <Trash2 aria-hidden />}
            {t("settings.remove_server", { executor: name })}
          </Button>
        )}
        {state === "unknown" && (
          <Button variant="outline" size="sm" disabled={checking} onClick={onCheckAgain}>
            {checking && <LoaderCircle aria-hidden className="animate-spin" />}
            {t("settings.check_again")}
          </Button>
        )}
      </div>
    </li>
  );
}

/**
 * IssueRelay's MCP control server (design 10.3), which lets Claude Code and Codex see how
 * IssueRelay stands and pause it: whether each executor has it, with the one click that registers
 * or removes it. A registration that could not be checked says so and can be checked again; there
 * is nothing to register without IssueRelay's command-line program, and a development build
 * has it only after `cargo build -p issuerelay-cli`.
 */
function ControlServers() {
  const { t } = useTranslation();
  const servers = useControlServers();
  const info = useAppInfo();
  const cliMissing = info.data !== undefined && info.data.cli_path === null;
  let rows: ReactNode;
  if (servers.isError && !servers.isFetching) {
    rows = <ErrorState error={servers.error} headline="settings.control_failed" onRetry={() => void servers.refetch()} />;
  } else if (servers.data === undefined) {
    rows = <LoadingState rows={2} />;
  } else {
    const found = servers.data;
    rows = (
      <ul className="flex flex-col gap-3">
        {(["claude", "codex"] as const).map((executor) => (
          <ControlServerRow
            key={executor}
            executor={executor}
            state={found[executor]}
            cliMissing={cliMissing}
            checking={servers.isFetching}
            onCheckAgain={() => void servers.refetch()}
          />
        ))}
      </ul>
    );
  }
  return (
    <>
      <p className="text-muted-foreground">{t("settings.control_server_intro")}</p>
      <p className="text-muted-foreground">{t("settings.control_server_note")}</p>
      {cliMissing && (
        <p className="text-muted-foreground">
          <WithCommands text={t("settings.cli_missing", { command: BUILD_CLI })} commands={[BUILD_CLI]} />
        </p>
      )}
      {rows}
    </>
  );
}

/** The four notifications, by the setting each switches. */
const NOTIFICATIONS: { key: keyof NotificationSettings; label: string }[] = [
  { key: "run_started", label: "settings.notify_run_started" },
  { key: "awaiting_acceptance", label: "settings.notify_awaiting_acceptance" },
  { key: "needs_info", label: "settings.notify_needs_info" },
  { key: "failed", label: "settings.notify_failed" },
];

/** The names of the label and the statuses, by the field of `TriggerNames` each is. */
const TRIGGER_NAMES: { key: keyof TriggerNames; label: string }[] = [
  { key: "label", label: "settings.label_name" },
  { key: "todo", label: "settings.state_todo" },
  { key: "in_progress", label: "settings.state_in_progress" },
  { key: "in_testing", label: "settings.state_in_testing" },
  { key: "done", label: "settings.state_done" },
];

/** The two numbers in a row of the group: the fields that say how often, how many and how long. */
const MONITORING: { field: CountField; label: string }[] = [
  { field: "poll_interval_secs", label: "settings.poll_interval" },
  { field: "max_concurrent", label: "settings.max_concurrent" },
  { field: "run_timeout_mins", label: "settings.run_timeout" },
  { field: "stall_timeout_mins", label: "settings.stall_timeout" },
];

const LOGS: { field: CountField; label: string }[] = [
  { field: "log_retention_days", label: "settings.log_retention" },
  { field: "log_max_total_mb", label: "settings.log_max_total" },
];

/**
 * The form of the settings. It edits a draft: what was changed is kept apart from the settings as
 * they are (so that what changes meanwhile, such as monitoring being paused, flows in and is never
 * written back over), and saving puts the changes into the settings as they are then. The
 * changes are checked as the engine checks them and saving waits for them to be right. Starting at
 * login and the control server act at once and are not part of the draft.
 */
function SettingsForm({ settings }: { settings: SettingsData }) {
  const { t } = useTranslation();
  const ids = useId();
  const id = (name: string) => `${ids}-${name}`;
  const base = draftOf(settings);
  const [edits, setEdits] = useState<Partial<Draft>>({});
  const [asking, setAsking] = useState(false);
  const save = useSaveSettings();
  const draft: Draft = { ...base, ...edits };
  const problems = problemsOf(draft);
  const dirty = !sameDraft(draft, base);

  const edit = <K extends keyof Draft>(field: K, value: Draft[K]) => setEdits((current) => ({ ...current, [field]: value }));

  const choosePermission = (level: PermissionProfile) => {
    // The first time full access is chosen the user is asked to understand what it means; until
    // then the choice is not made.
    if (level === "full" && !draft.full_access_acknowledged) setAsking(true);
    else edit("default_permission", level);
  };
  const understood = () => {
    setAsking(false);
    setEdits((current) => ({ ...current, default_permission: "full", full_access_acknowledged: true }));
  };

  const submit = () => {
    const next = settingsOf(settings, draft);
    if (next === null || save.isPending) return;
    const saved = draft;
    save.mutate(next, {
      onSuccess: () => {
        toast.success(t("common.saved"));
        // What was typed while the save ran is still to be saved.
        setEdits((current) => unsaved(current, saved));
      },
    });
  };

  const count = ({ field, label }: { field: CountField; label: string }) => (
    <TextField
      key={field}
      id={id(field)}
      label={t(label)}
      type="number"
      min={MINIMUM[field]}
      value={draft[field]}
      problem={problems[field]}
      onChange={(value) => edit(field, value)}
    />
  );

  return (
    <>
      <div className="flex flex-col gap-4">
        <Group title={t("settings.group.general")}>
          <LanguageField
            id={id("language")}
            label={t("settings.language")}
            value={draft.language}
            first={{ value: "system", label: t("settings.follow_system") }}
            onChange={(value) => edit("language", value)}
          />
          <LanguageField
            id={id("comment-language")}
            label={t("settings.comment_language")}
            value={draft.comment_language}
            first={{ value: "follow-ui", label: t("settings.follow_ui") }}
            hint={t("settings.comment_language_hint")}
            onChange={(value) => edit("comment_language", value)}
          />
        </Group>

        <Group title={t("settings.group.monitoring")}>
          <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-4">{MONITORING.map(count)}</div>
        </Group>

        <Group title={t("settings.group.defaults")}>
          <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-4">
            <Field label={t("settings.default_executor")} htmlFor={id("executor")}>
              <Select value={draft.default_executor} onValueChange={(value) => edit("default_executor", value as ExecutorKind)}>
                <SelectTrigger id={id("executor")} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="claude">{t("executor.claude")}</SelectItem>
                  <SelectItem value="codex">{t("executor.codex")}</SelectItem>
                </SelectContent>
              </Select>
            </Field>
            <Field label={t("settings.default_permission")} htmlFor={id("permission")}>
              <Select value={draft.default_permission} onValueChange={(value) => choosePermission(value as PermissionProfile)}>
                <SelectTrigger id={id("permission")} className="w-full" aria-describedby={id("permission-hint")}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="full">{t("permission.full")}</SelectItem>
                  <SelectItem value="auto">{t("permission.auto")}</SelectItem>
                  <SelectItem value="personal">{t("permission.personal")}</SelectItem>
                </SelectContent>
              </Select>
            </Field>
          </div>
          <Hint id={id("permission-hint")}>{t(`permission.${draft.default_permission}_hint`)}</Hint>
        </Group>

        <Group title={t("settings.group.prompt")}>
          <Field label={t("settings.prompt_template")} htmlFor={id("template")}>
            <Textarea
              id={id("template")}
              value={draft.prompt_template}
              onChange={(event) => edit("prompt_template", event.target.value)}
              aria-describedby={id("template-hint")}
              rows={5}
              className="max-h-64"
            />
            <Hint id={id("template-hint")}>{t("settings.prompt_hint")}</Hint>
          </Field>
        </Group>

        <Group title={t("settings.group.notifications")}>
          <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-x-6 gap-y-3">
            {NOTIFICATIONS.map(({ key, label }) => (
              <SwitchRow
                key={key}
                id={id(`notify-${key}`)}
                label={t(label)}
                checked={draft.notifications[key]}
                onChange={(on) => edit("notifications", { ...draft.notifications, [key]: on })}
              />
            ))}
          </div>
        </Group>

        <Group title={t("settings.group.system")}>
          <Autostart />
          <CliOnPath />
          <SwitchRow
            id={id("auto-update")}
            label={t("settings.auto_update")}
            note={t("settings.auto_update_note")}
            checked={draft.auto_update}
            onChange={(on) => edit("auto_update", on)}
          />
        </Group>

        <Group title={t("settings.group.mcp")}>
          <ControlServers />
        </Group>

        <Group title={t("settings.group.advanced")}>
          <Part title={t("settings.trigger_names")} />
          <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-4">
            {TRIGGER_NAMES.map(({ key, label }) => (
              <TextField
                key={key}
                id={id(`trigger-${key}`)}
                label={t(label)}
                value={draft.trigger[key]}
                problem={problems[`trigger.${key}`]}
                onChange={(value) => edit("trigger", { ...draft.trigger, [key]: value })}
              />
            ))}
          </div>
          <Separator />
          <Part title={t("settings.linear_access")} />
          <ApiKeyForm />
          <Separator />
          <Part title={t("settings.logs")} />
          <div className="grid grid-cols-[repeat(auto-fit,minmax(14rem,1fr))] gap-4">{LOGS.map(count)}</div>
        </Group>
      </div>

      {/* Stays at the foot of the window while the page scrolls under it. */}
      <div
        data-slot="save-bar"
        className="sticky bottom-0 z-10 -mx-(--page-padding) mt-6 flex items-center justify-end gap-3 border-t bg-background/95 px-(--page-padding) py-3 backdrop-blur"
      >
        <p aria-live="polite" className="text-sm text-muted-foreground">
          {dirty && t("settings.unsaved")}
        </p>
        <Button disabled={save.isPending || Object.keys(problems).length > 0} onClick={submit}>
          {save.isPending && <LoaderCircle aria-hidden className="animate-spin" />}
          {t("common.save")}
        </Button>
      </div>

      <FullAccessWarning open={asking} onConfirm={understood} onCancel={() => setAsking(false)} />
    </>
  );
}

/**
 * The settings (design 9.2): one form of grouped cards with the Save button fixed at the foot.
 * Language, monitoring, defaults, the prompt template, notifications and Advanced are saved
 * together; starting at login and the MCP control server act at once. The settings are read
 * here, and a form is made only once they are.
 */
export default function Settings() {
  const { t } = useTranslation();
  const settings = useSettings();
  let body: ReactNode;
  if (settings.data !== undefined) {
    // A later read that failed leaves the settings that were read: the form, with what was typed
    // in it, is not taken away for that.
    body = <SettingsForm settings={settings.data} />;
  } else if (settings.isError && !settings.isFetching) {
    body = <ErrorState error={settings.error} headline="settings.load_failed" onRetry={() => void settings.refetch()} />;
  } else {
    body = <LoadingState rows={6} />;
  }
  return (
    <>
      <PageHeader title={t("nav.settings")} description={t("settings.description")} />
      {body}
    </>
  );
}

import { Copy, LoaderCircle, LogIn, Plus } from "lucide-react";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { CheckRow } from "@/components/CheckRow";
import { DetailLine, LoadingState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { WithCommands } from "@/components/WithCommands";
import { useLinearServerNamed, useSetupAction } from "@/hooks/queries";
import { useCopyText } from "@/hooks/useCopyText";
import { cleanDisplayText } from "@/lib/clean";
import { describeError } from "@/lib/errors";
import type { ExecutorKind, ExecutorReport } from "@/lib/types";

/**
 * Where each executor's own instructions for installing it are. They are shown, to be copied, and
 * never opened: the app opens Linear's pages and no others (`open_linear`).
 */
const INSTALL_URL: Record<ExecutorKind, string> = {
  claude: "https://docs.anthropic.com/en/docs/claude-code/setup",
  codex: "https://developers.openai.com/codex/cli",
};

/** What to run in a terminal to log in to the executor. */
const LOGIN_COMMAND: Record<ExecutorKind, string> = { claude: "claude", codex: "codex login" };

/** What finds Linear's server under whatever name it was added with, and logs in to it. */
const NAMED_COMMANDS: Record<ExecutorKind, { list: string; login: string }> = {
  claude: { list: "claude mcp list", login: "claude mcp login <name>" },
  codex: { list: "codex mcp list", login: "codex mcp login <name>" },
};

/** Where to get the executor: its official instructions, as an address to copy. */
function InstallHint({ executor, name }: { executor: ExecutorKind; name: string }) {
  const { t } = useTranslation();
  const copy = useCopyText();
  const address = INSTALL_URL[executor];
  return (
    <div className="flex flex-col gap-1.5">
      <p>{t("doctor.install_hint", { executor: name })}</p>
      <div className="flex flex-wrap items-center gap-2">
        <code className="min-w-0 rounded bg-muted px-1.5 py-0.5 font-mono text-xs break-all select-text">{address}</code>
        <Button variant="outline" size="xs" onClick={() => copy(address)}>
          <Copy aria-hidden />
          {t("common.copy")}
        </Button>
      </div>
    </div>
  );
}

/**
 * What to do about Linear's MCP server not being connected in an installed executor. The one-click
 * login works only for a server that was added under the name the setup commands use
 * (`linear-server` for Claude Code, `linear` for Codex), so the executor is asked which it is.
 * Under that name there is the login; under another, the add command and the way to log in by hand
 * (`mcp login` with the name `mcp list` shows). While it is not known, or when it could not be
 * found out, there is no guess: no button, and the commands are there to read.
 */
function LinearFix({ report, name, onFixed }: { report: ExecutorReport; name: string; onFixed?: () => void }) {
  const { t } = useTranslation();
  const named = useLinearServerNamed(report.kind);
  const setup = useSetupAction();
  const commands = NAMED_COMMANDS[report.kind];
  // The setup action marks the check outdated, which is read again where it is shown; `onFixed`
  // is for a host that shows it some other way.
  const run = (kind: "add_linear_mcp" | "login_linear_mcp") => setup.mutate({ kind, executor: report.kind }, { onSuccess: onFixed });
  const manual = (
    <p className="text-muted-foreground">
      <WithCommands text={t("doctor.other_name", { list: commands.list, login: commands.login })} commands={[commands.list, commands.login]} />
    </p>
  );

  if (named.isError && !named.isFetching) {
    const failure = describeError(named.error, t, "doctor.server_check_failed");
    return (
      <div className="flex flex-col items-start gap-1.5">
        <p className="font-medium text-destructive">{failure.headline}</p>
        {failure.detail !== null && <DetailLine text={failure.detail} />}
        <Button variant="outline" size="sm" onClick={() => void named.refetch()}>
          {t("common.retry")}
        </Button>
        {manual}
      </div>
    );
  }
  if (named.data === undefined) return <LoadingState rows={1} />;
  if (named.data) {
    return (
      <div className="flex flex-col items-start gap-1.5">
        <Button size="sm" disabled={setup.isPending} onClick={() => run("login_linear_mcp")}>
          {setup.isPending ? <LoaderCircle aria-hidden className="animate-spin" /> : <LogIn aria-hidden />}
          {t("doctor.login_linear", { executor: name })}
        </Button>
        <p className="text-xs text-muted-foreground">{t("doctor.login_note")}</p>
      </div>
    );
  }
  return (
    <div className="flex flex-col items-start gap-1.5">
      <Button size="sm" disabled={setup.isPending} onClick={() => run("add_linear_mcp")}>
        {setup.isPending ? <LoaderCircle aria-hidden className="animate-spin" /> : <Plus aria-hidden />}
        {t("doctor.add_linear", { executor: name })}
      </Button>
      {manual}
    </div>
  );
}

/**
 * One executor's part of the environment check (design 6.4, 8): whether it is installed (and which
 * version), logged in and connected to Linear, each as a translated title from the report's
 * booleans, the engine's own lines under them, and the fix for what is missing. A fix that works
 * is followed by a new check (the setup action makes it outdated); `onFixed` also tells the
 * host, for one that does not show the check live (the first-run guide).
 *
 * The executor's name stands in the card's heading only, as the whole text of one element.
 */
export function ExecutorCheck({ report, onFixed }: { report: ExecutorReport; onFixed?: () => void }) {
  const { t } = useTranslation();
  const titleId = useId();
  const name = t(`executor.${report.kind}`);
  const installed = report.program !== null;
  const version = report.version === null ? "" : cleanDisplayText(report.version).trim();
  const program = report.program === null ? "" : cleanDisplayText(report.program);
  const login = LOGIN_COMMAND[report.kind];
  return (
    <section aria-labelledby={titleId}>
      <Card size="sm" className="h-full">
        <div className="px-(--card-spacing)">
          <h3 id={titleId} className="text-base font-medium">
            {name}
          </h3>
        </div>
        <CardContent>
          <ul className="flex flex-col gap-1.5">
            <CheckRow ok={installed}>
              <span className="flex flex-wrap items-baseline gap-x-2">
                <span>{installed ? t("doctor.installed") : t("doctor.not_installed")}</span>
                {version !== "" && <span className="font-mono text-xs text-muted-foreground">{version}</span>}
              </span>
              {program !== "" && <span className="font-mono text-xs break-all text-muted-foreground">{program}</span>}
            </CheckRow>
            <CheckRow ok={report.logged_in}>
              <span>{report.logged_in ? t("doctor.logged_in") : t("doctor.not_logged_in")}</span>
            </CheckRow>
            <CheckRow ok={report.linear_mcp}>
              <span>{report.linear_mcp ? t("doctor.linear_mcp_ok") : t("doctor.linear_mcp_missing")}</span>
            </CheckRow>
          </ul>
          {report.problems.length > 0 && (
            <div className="flex flex-col gap-1">
              {report.problems.map((problem, index) => (
                <DetailLine key={index} text={problem} />
              ))}
            </div>
          )}
          {!installed && <InstallHint executor={report.kind} name={name} />}
          {installed && !report.logged_in && (
            <p>
              <WithCommands text={t("doctor.login_hint", { command: login })} commands={[login]} />
            </p>
          )}
          {installed && !report.linear_mcp && <LinearFix report={report} name={name} onFixed={onFixed} />}
        </CardContent>
      </Card>
    </section>
  );
}

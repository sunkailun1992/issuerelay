import { ChevronRight, CircleX, SquareTerminal, Wrench } from "lucide-react";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import type { TimelineEntry } from "@/lib/timeline";
import { cn } from "@/lib/utils";

/** A command or tool name this long, or over several lines, is worth opening even when it has no output to show. */
const LONG_NAME = 60;

/** Marks something that failed in words for a screen reader as well as in the state colour. */
function FailedMark() {
  const { t } = useTranslation();
  return <CircleX role="img" aria-label={t("state.failed")} className="size-4 shrink-0 text-state-failed" />;
}

/** The frame of a card; a failed one takes the failure colour (a border and a faint tint). */
const card = (failed: boolean | undefined) => cn("rounded-lg border bg-card text-card-foreground", failed && "border-state-failed/30 bg-state-failed/5");

/** What both kinds of card show for a text that can be long: one line, with an ellipsis, until the card is open. */
const NAME =
  "min-w-0 flex-1 truncate font-mono text-xs group-data-[state=open]/call:overflow-visible group-data-[state=open]/call:wrap-anywhere group-data-[state=open]/call:whitespace-pre-wrap";

/** The text of an input or an output, in a box of its own. */
function Detail({ text }: { text: string }) {
  return (
    <pre className="max-h-80 overflow-auto rounded-md bg-muted px-3 py-2 font-mono text-xs wrap-anywhere whitespace-pre-wrap select-text">{text}</pre>
  );
}

/**
 * A tool call or a command. Its header is the tool's name or the command line; what went in and
 * what came out is below, once the card is open. A card with nothing to show but its name is not a
 * button.
 */
function CallCard({ entry }: { entry: TimelineEntry }) {
  const Icon = entry.kind === "command" ? SquareTerminal : Wrench;
  const openable = entry.detail !== undefined || entry.text.length > LONG_NAME || entry.text.includes("\n");
  const header = (
    <>
      <Icon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <span className={NAME}>{entry.text}</span>
      {entry.failed && <FailedMark />}
      {openable && <ChevronRight aria-hidden className="size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]/call:rotate-90" />}
    </>
  );
  if (!openable) return <div className={cn(card(entry.failed), "group/call flex items-center gap-2 px-3 py-2")}>{header}</div>;
  return (
    <Collapsible className={cn(card(entry.failed), "group/call")}>
      <CollapsibleTrigger className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
        {header}
      </CollapsibleTrigger>
      {entry.detail !== undefined && (
        <CollapsibleContent>
          <div className="border-t px-3 py-2">
            <Detail text={entry.detail} />
          </div>
        </CollapsibleContent>
      )}
    </Collapsible>
  );
}

/** The end of the run: the AI's last word, in a card of its own. */
function ResultCard({ entry }: { entry: TimelineEntry }) {
  const { t } = useTranslation();
  const label = useId();
  return (
    <div role="group" aria-labelledby={label} className={cn(card(entry.failed), "px-3 py-2")}>
      <div className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <span id={label}>{t("timeline.result")}</span>
        {entry.failed && <FailedMark />}
      </div>
      <p className="wrap-anywhere whitespace-pre-wrap">{entry.text}</p>
    </div>
  );
}

/** Where a labelled line came from, by the key of the label that says so. */
const LINE_LABEL = { error: "timeline.error", stderr: "timeline.stderr", note: "timeline.note" } as const;

/**
 * A line of the executor's error output, of an error it reported, or of IssueRelay's own notes
 * (a start that failed): small, monospace, with the name of where it came from.
 */
function LabelledLine({ kind, text }: { kind: keyof typeof LINE_LABEL; text: string }) {
  const { t } = useTranslation();
  const label = useId();
  return (
    <div role="group" aria-labelledby={label} className="flex items-baseline gap-2 font-mono text-xs">
      <span id={label} className={cn("shrink-0 rounded bg-muted px-1.5 py-0.5", kind === "error" ? "text-state-failed" : "text-muted-foreground")}>
        {t(LINE_LABEL[kind])}
      </span>
      <span className={cn("min-w-0 wrap-anywhere whitespace-pre-wrap", kind !== "error" && "text-muted-foreground")}>{text}</span>
    </div>
  );
}

function SessionStart({ model }: { model: string }) {
  const { t } = useTranslation();
  return (
    <p className="flex flex-wrap items-baseline gap-x-2 text-muted-foreground">
      <span>{t("timeline.session_started")}</span>
      {model !== "" && <span className="font-mono text-xs">{model}</span>}
    </p>
  );
}

function Row({ entry }: { entry: TimelineEntry }) {
  switch (entry.kind) {
    case "system":
      return <SessionStart model={entry.text} />;
    case "message":
      return <p className="wrap-anywhere whitespace-pre-wrap">{entry.text}</p>;
    case "tool":
    case "command":
      return <CallCard entry={entry} />;
    case "result":
      return <ResultCard entry={entry} />;
    case "error":
    case "stderr":
    case "note":
      return <LabelledLine kind={entry.kind} text={entry.text} />;
  }
}

/**
 * A run's timeline (design 9.2): what the AI said as text, each tool call and command as a card
 * that opens, the end of the run as a card of its own. Everything in it was cleaned when the
 * entries were made (`parseTimeline`) and is shown as text, never as markup.
 */
export function RunTimeline({ entries }: { entries: TimelineEntry[] }) {
  return (
    <ol className="flex flex-col gap-3">
      {entries.map((entry, index) => (
        // The log only grows at its end, so a row's place in it is its identity: a card that was
        // opened stays open while the log is read again.
        <li key={index} className="min-w-0">
          <Row entry={entry} />
        </li>
      ))}
    </ol>
  );
}

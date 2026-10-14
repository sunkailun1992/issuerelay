import { cleanDisplayLines, cleanDisplayText } from "./clean";
import type { ExecutorKind } from "./types";

/**
 * What a row of a run's timeline is. `note` is IssueRelay's own line in the log (a start that
 * failed, say), `stderr` a line the executor wrote to its error output or one the log cannot read,
 * `files` the files Codex changed.
 */
export type TimelineKind = "message" | "tool" | "command" | "files" | "result" | "error" | "stderr" | "note" | "system";

/** A file Codex changed and how: added, deleted or changed, or `null` for a kind the panel does not know. */
export interface FileChange {
  kind: "add" | "delete" | "update" | null;
  path: string;
}

/**
 * One row of the timeline the panel builds from a run's log (design 6.1, 9.2). Its texts are
 * cleaned (`cleanDisplayText`) and ready to show as they are. For `system` the text is the model's
 * name, or empty; the panel words the rest. A `tool` or `command` carries what went in and what
 * came out as `detail`, and `failed` when it did not work. A `files` row lists its `changes` (its
 * text is their paths, one per line).
 */
export interface TimelineEntry {
  kind: TimelineKind;
  text: string;
  detail?: string;
  failed?: boolean;
  changes?: FileChange[];
}

/**
 * The most characters of a tool's input or result, or of a command line, a row keeps, counted once
 * cleaned; the raw log has the rest. A card shows all of it once opened.
 */
const DETAIL_LIMIT = 2000;

type Json = Record<string, unknown>;

function asRecord(value: unknown): Json | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Json) : null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

/** A name, a command line or a note: cleaned, on one line. */
function oneLine(text: string): string {
  return cleanDisplayText(text).trim();
}

/** What the AI or an executor wrote over several lines: each line cleaned, the blank space around it gone. */
function manyLines(text: string): string {
  return cleanDisplayLines(text).trim();
}

/**
 * `text` cut to `DETAIL_LIMIT` characters with an ellipsis. A character that takes two UTF-16
 * units is cut whole or not at all.
 */
function capped(text: string): string {
  if (text.length <= DETAIL_LIMIT) return text;
  let end = DETAIL_LIMIT;
  const last = text.charCodeAt(end - 1);
  if (last >= 0xd800 && last <= 0xdbff) end -= 1;
  return `${text.slice(0, end)}…`;
}

/**
 * A tool's input or output: each line cleaned, the trailing blank space gone (the indent of the
 * first line is its own), and cut to `DETAIL_LIMIT` characters (`capped`).
 */
function detailOf(text: string): string {
  return capped(cleanDisplayLines(text).trimEnd());
}

/** A command line over one or more lines: cleaned like the AI's text and cut like a tool's output. */
function commandLine(text: string): string {
  return capped(manyLines(text));
}

/** The detail of a row: what went in, then what came out below it, whichever there is. */
function joined(...parts: string[]): string {
  return parts.filter((part) => part !== "").join("\n\n");
}

function entry(kind: TimelineKind, text: string, { detail = "", failed = false } = {}): TimelineEntry {
  const made: TimelineEntry = { kind, text };
  if (detail !== "") made.detail = detail;
  if (failed) made.failed = true;
  return made;
}

/** A call whose result may still come. */
interface Call {
  made: TimelineEntry;
  /** What went in, already a detail (empty for a command: the command line is the row's text). */
  input: string;
  answered: boolean;
}

interface Context {
  entries: TimelineEntry[];
  /** The calls Claude has made, by the id its results name them with. */
  calls: Map<string, Call>;
  /** The call made last. */
  latest: Call | null;
}

function addCall(ctx: Context, id: string | null, made: TimelineEntry, input: string): void {
  ctx.entries.push(made);
  const call = { made, input, answered: false };
  if (id !== null) ctx.calls.set(id, call);
  ctx.latest = call;
}

/** The parts of a message's content that are objects; the content of anything else is none. */
function blocksOf(event: Json): Json[] {
  const content = asRecord(event.message)?.content;
  return Array.isArray(content) ? content.map(asRecord).filter((block) => block !== null) : [];
}

/** The text of a tool's result: a string, or the text parts of a list of parts (images and the like are left out). */
function resultText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => (typeof part === "string" ? part : asString(asRecord(part)?.text)))
    .filter((part) => part !== null)
    .join("\n");
}

function addToolUse(ctx: Context, block: Json): void {
  const name = asString(block.name);
  if (name === null || oneLine(name) === "") return;
  const id = asString(block.id);
  const command = name === "Bash" ? asString(asRecord(block.input)?.command) : null;
  const line = command === null ? "" : commandLine(command);
  if (line !== "") {
    addCall(ctx, id, entry("command", line), "");
    return;
  }
  const input = block.input === undefined || block.input === null ? "" : detailOf(JSON.stringify(block.input));
  addCall(ctx, id, entry("tool", oneLine(name), { detail: input }), input);
}

/**
 * A result belongs to the call its id names; one without an id, or with one nobody was given, to
 * the call made last if that has no result yet; otherwise it is a row of its own. A command's
 * result is its output and replaces the detail (the command line is the row's text); another
 * tool's result comes below the input.
 */
function addToolResult(ctx: Context, block: Json): void {
  const id = asString(block.tool_use_id);
  const last = ctx.latest !== null && !ctx.latest.answered ? ctx.latest : undefined;
  const call = (id === null ? undefined : ctx.calls.get(id)) ?? last;
  const output = detailOf(resultText(block.content));
  const failed = block.is_error === true;
  if (call === undefined) {
    if (output !== "" || failed) ctx.entries.push(entry("tool", "tool_result", { detail: output, failed }));
    return;
  }
  call.answered = true;
  const detail = call.made.kind === "command" ? output : joined(call.input, output);
  if (detail === "") delete call.made.detail;
  else call.made.detail = detail;
  if (failed) call.made.failed = true;
}

function claude(event: Json, ctx: Context): void {
  switch (event.type) {
    case "system": {
      if (event.subtype !== "init") return;
      const model = asString(event.model);
      ctx.entries.push({ kind: "system", text: model === null ? "" : oneLine(model) });
      return;
    }
    case "assistant":
      for (const block of blocksOf(event)) {
        if (block.type === "text") {
          const text = manyLines(asString(block.text) ?? "");
          if (text !== "") ctx.entries.push({ kind: "message", text });
        } else if (block.type === "tool_use") {
          addToolUse(ctx, block);
        }
      }
      return;
    case "user":
      for (const block of blocksOf(event)) {
        if (block.type === "tool_result") addToolResult(ctx, block);
      }
      return;
    case "result": {
      // A run that ended badly may have no text, only a kind of ending ("error_max_turns").
      const text = manyLines(asString(event.result) ?? "") || oneLine(asString(event.subtype) ?? "");
      if (text !== "") ctx.entries.push(entry("result", text, { failed: event.is_error === true }));
      return;
    }
  }
}

function codexItem(item: Json, ctx: Context): void {
  switch (item.type) {
    case "agent_message": {
      const text = manyLines(asString(item.text) ?? "");
      if (text !== "") ctx.entries.push({ kind: "message", text });
      return;
    }
    case "command_execution": {
      const command = commandLine(asString(item.command) ?? "");
      if (command === "") return;
      // The exit code says it; a command that has none (it was stopped) is judged by its status.
      const failed = typeof item.exit_code === "number" ? item.exit_code !== 0 : item.status === "failed";
      ctx.entries.push(entry("command", command, { detail: detailOf(asString(item.aggregated_output) ?? ""), failed }));
      return;
    }
    case "mcp_tool_call": {
      const tool = oneLine(asString(item.tool) ?? "");
      if (tool === "") return;
      const server = oneLine(asString(item.server) ?? "");
      const input = item.arguments === undefined || item.arguments === null ? "" : detailOf(JSON.stringify(item.arguments));
      const error = typeof item.error === "string" ? item.error : asString(asRecord(item.error)?.message);
      const output = detailOf(error ?? resultText(asRecord(item.result)?.content));
      const failed = (item.error !== undefined && item.error !== null) || item.status === "failed";
      ctx.entries.push(entry("tool", server === "" ? tool : `${server}.${tool}`, { detail: joined(input, output), failed }));
      return;
    }
    case "file_change": {
      const changes = (Array.isArray(item.changes) ? item.changes : [])
        .map(asRecord)
        .filter((change) => change !== null)
        .map((change): FileChange => ({ kind: changeKind(change.kind), path: oneLine(asString(change.path) ?? "") }))
        .filter((change) => change.path !== "");
      if (changes.length === 0) return;
      const made: TimelineEntry = { kind: "files", text: changes.map((change) => change.path).join("\n"), changes };
      if (item.status === "failed") made.failed = true;
      ctx.entries.push(made);
      return;
    }
    case "error": {
      const message = manyLines(asString(item.message) ?? "");
      if (message !== "") ctx.entries.push({ kind: "error", text: message });
      return;
    }
  }
}

/** What happened to a file, as Codex names it; another word is a kind the panel does not know. */
function changeKind(kind: unknown): FileChange["kind"] {
  return kind === "add" || kind === "delete" || kind === "update" ? kind : null;
}

function codex(event: Json, ctx: Context): void {
  switch (event.type) {
    case "item.completed": {
      const item = asRecord(event.item);
      if (item !== null) codexItem(item, ctx);
      return;
    }
    case "error": {
      const message = manyLines(asString(event.message) ?? "");
      if (message !== "") ctx.entries.push({ kind: "error", text: message });
      return;
    }
    case "turn.failed": {
      const message = manyLines(asString(asRecord(event.error)?.message) ?? "");
      ctx.entries.push({ kind: "error", text: message === "" ? "turn.failed" : message });
      return;
    }
  }
}

/** IssueRelay's own line in the log: `{"stream":"stderr"|"runner","text":…}`. */
function stream(event: Json, ctx: Context): void {
  const text = oneLine(asString(event.text) ?? "");
  if (text !== "") ctx.entries.push({ kind: event.stream === "runner" ? "note" : "stderr", text });
}

/** The line as an object, or `null` when it is not JSON or not an object. */
function parseLine(line: string): Json | null {
  try {
    return asRecord(JSON.parse(line));
  } catch {
    return null;
  }
}

/**
 * The readable timeline of a run from the lines of its log: each line is JSON, the executor's own
 * event (read in `executor`'s dialect, or in both when it is not known) or one of IssueRelay's
 * (`stream`). A line that is not an event object is kept as it is, as `stderr`; an event that says
 * nothing a reader needs (the start of a turn, usage, a thought) is left out. The rows come in the
 * order of the log. Every text in them is cleaned.
 *
 * The shapes are those of Claude Code 2.1.285 (`--output-format stream-json`) and of Codex's
 * `exec --json`, as sampled in crates/core/tests/fixtures (2026-10-10). Those samples hold no
 * `tool_result`, no failing Codex item and none of Codex's `file_change` and `error` items, so
 * these shapes are taken from the tools' own descriptions of their events (for Codex, the item
 * types of codex-rs `exec_events.rs`, read on 2026-10-14) and wait for a real run's log in the
 * live acceptance.
 */
export function parseTimeline(lines: string[], executor: ExecutorKind | null): TimelineEntry[] {
  const ctx: Context = { entries: [], calls: new Map(), latest: null };
  for (const line of lines) {
    const event = parseLine(line);
    if (event === null) {
      const text = oneLine(line);
      if (text !== "") ctx.entries.push({ kind: "stderr", text });
    } else if (typeof event.type === "string") {
      if (executor !== "codex") claude(event, ctx);
      if (executor !== "claude") codex(event, ctx);
    } else if (typeof event.stream === "string") {
      stream(event, ctx);
    }
  }
  return ctx.entries;
}

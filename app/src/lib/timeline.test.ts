import { expect, test } from "vitest";
// The streams core itself is tested against: read from there, never copied (vite.config.ts lets a
// test run read them).
import claudeStream from "../../../crates/core/tests/fixtures/claude-stream.jsonl?raw";
import codexExec from "../../../crates/core/tests/fixtures/codex-exec.jsonl?raw";
import { parseTimeline } from "./timeline";

const linesOf = (text: string) => text.split("\n").filter(Boolean);

// Characters that must never reach the screen are built here and not written as escapes in the
// source: a tool that writes this file could turn an escape into the character itself.
const ESC = String.fromCharCode(0x1b);
const RLO = String.fromCharCode(0x202e);
const ZERO_WIDTH_SPACE = String.fromCharCode(0x200b);

/** A line of Claude's stream: an assistant message with these content blocks. */
const assistant = (...content: unknown[]) => JSON.stringify({ type: "assistant", message: { content } });
/** A line of Claude's stream: the results of tool calls. */
const toolResults = (...content: unknown[]) => JSON.stringify({ type: "user", message: { content } });
/** A line of Codex's stream: a finished item. */
const completed = (item: object) => JSON.stringify({ type: "item.completed", item });

test("a real Claude stream becomes a readable timeline", () => {
  const entries = parseTimeline(linesOf(claudeStream), "claude");
  expect(entries[0].kind).toBe("system");
  expect(entries.some((e) => e.kind === "tool" || e.kind === "command")).toBe(true);
  expect(entries.at(-1)?.kind).toBe("result");
  expect(entries.filter((e) => e.kind === "message").length).toBeGreaterThan(0);
});

test("a real Codex stream becomes a readable timeline", () => {
  const entries = parseTimeline(linesOf(codexExec), "codex");
  expect(entries.some((e) => e.kind === "message")).toBe(true);
  expect(entries.every((e) => e.text.length > 0)).toBe(true);
});

test("in the real Claude stream the session, the Linear call, the answer and the result each have their row", () => {
  const entries = parseTimeline(linesOf(claudeStream), "claude");
  expect(entries.map((e) => e.kind)).toEqual(["system", "tool", "message", "result"]);
  expect(entries[0]).toEqual({ kind: "system", text: "claude-opus-5-5" });
  // The call has no result in the stream: only its input is there.
  expect(entries[1]).toEqual({
    kind: "tool",
    text: "mcp__linear-server__get_issue",
    detail: '{"id":"DEMO-1","includeRelations":true}',
  });
  expect(entries[2].text).toContain("hello.txt");
  // The text keeps its lines (the answer is several paragraphs and a list).
  expect(entries[2].text.split("\n").length).toBeGreaterThan(5);
  expect(entries[3].text).toBe(entries[2].text);
  expect(entries[3].failed).toBeUndefined();
});

test("in the real Codex stream the Linear call, the command and the answer each have their row", () => {
  const entries = parseTimeline(linesOf(codexExec), "codex");
  expect(entries.map((e) => e.kind)).toEqual(["tool", "command", "message"]);
  expect(entries[0].text).toBe("linear.get_issue");
  // The call's arguments, then what it returned, below them.
  expect(entries[0].detail).toMatch(/^\{"id":"DEMO-2","fields":\["id","title",/);
  expect(entries[0].detail).toContain("\n\n");
  expect(entries[0].detail).toContain("Sandbox: add usage line to README");
  expect(entries[0].failed).toBeUndefined();
  expect(entries[1]).toEqual({ kind: "command", text: "/bin/zsh -lc pwd", detail: "~/issuerelay-sandbox" });
  expect(entries[2].text).toContain("README.md");
});

test("a command's result replaces its detail and failures are marked", () => {
  const lines = [
    JSON.stringify({ type: "assistant", message: { content: [{ type: "tool_use", id: "t1", name: "Bash", input: { command: "cargo test" } }] } }),
    JSON.stringify({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "t1", content: "1 failed", is_error: true }] } }),
    JSON.stringify({ type: "result", result: "could not fix the test", is_error: true }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "command", text: "cargo test", detail: "1 failed", failed: true },
    { kind: "result", text: "could not fix the test", failed: true },
  ]);
});

test("another tool's result is added below its input", () => {
  const lines = [
    JSON.stringify({ type: "assistant", message: { content: [{ type: "tool_use", id: "t2", name: "Read", input: { file_path: "src/lib.rs" } }] } }),
    JSON.stringify({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "t2", content: "fn main() {}", is_error: false }] } }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "tool", text: "Read", detail: '{"file_path":"src/lib.rs"}\n\nfn main() {}' },
  ]);
});

test("Codex commands carry their output and exit code", () => {
  const lines = [
    JSON.stringify({ type: "item.started", item: { type: "command_execution", command: "git push" } }),
    JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: "git push", aggregated_output: "rejected", exit_code: 1 } }),
    JSON.stringify({ type: "error", message: "stream disconnected" }),
  ];
  expect(parseTimeline(lines, "codex")).toEqual([
    { kind: "command", text: "git push", detail: "rejected", failed: true },
    { kind: "error", text: "stream disconnected" },
  ]);
});

test("unreadable lines and IssueRelay's own notes stay visible, cleaned", () => {
  const lines = [
    "not json at all",
    JSON.stringify({ stream: "stderr", text: `${ESC}[31mwarning${ESC}[0m: slow` }),
    JSON.stringify({ stream: "runner", text: "spawn failed: No such file or directory" }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "stderr", text: "not json at all" },
    { kind: "stderr", text: "warning: slow" },
    { kind: "note", text: "spawn failed: No such file or directory" },
  ]);
});

test("text from the AI is cleaned before it is shown", () => {
  // U+202E (right-to-left override) is removed, U+001B [2J (clear the screen) too.
  const lines = [JSON.stringify({ type: "assistant", message: { content: [{ type: "text", text: `done${RLO}${ESC}[2J` }] } })];
  expect(parseTimeline(lines, "claude")).toEqual([{ kind: "message", text: "done" }]);
});

test("long details are cut", () => {
  const lines = [
    JSON.stringify({ type: "item.completed", item: { type: "command_execution", command: "cat big", aggregated_output: "x".repeat(5000), exit_code: 0 } }),
  ];
  const [entry] = parseTimeline(lines, "codex");
  expect(entry.detail!.length).toBeLessThanOrEqual(2001);
});

// ---- the rest of the table: what each event becomes -------------------------------------------

test("a session start names the model when it has one", () => {
  const withModel = JSON.stringify({ type: "system", subtype: "init", model: "claude-sonnet-5-5" });
  const withoutModel = JSON.stringify({ type: "system", subtype: "init" });
  expect(parseTimeline([withModel, withoutModel], "claude")).toEqual([
    { kind: "system", text: "claude-sonnet-5-5" },
    { kind: "system", text: "" },
  ]);
});

test("the model's name is cleaned like any other text from the executor", () => {
  const init = JSON.stringify({ type: "system", subtype: "init", model: `claude${RLO}-x${ESC}[2J` });
  expect(parseTimeline([init], "claude")).toEqual([{ kind: "system", text: "claude-x" }]);
});

test("an assistant message gives one row for each of its parts, in order", () => {
  const line = assistant(
    { type: "text", text: "First I look." },
    { type: "tool_use", id: "a", name: "Read", input: { file_path: "a.rs" } },
    { type: "text", text: "Then I change it." },
    { type: "tool_use", id: "b", name: "Bash", input: { command: "ls" } },
  );
  expect(parseTimeline([line], "claude")).toEqual([
    { kind: "message", text: "First I look." },
    { kind: "tool", text: "Read", detail: '{"file_path":"a.rs"}' },
    { kind: "message", text: "Then I change it." },
    { kind: "command", text: "ls" },
  ]);
});

test("a message keeps its lines, each cleaned, and loses the blank space around it", () => {
  const text = `\n\nDone:\r\n- ${ZERO_WIDTH_SPACE}one\n- two${ESC}[0m\n\n`;
  expect(parseTimeline([assistant({ type: "text", text })], "claude")).toEqual([{ kind: "message", text: "Done:\n- one\n- two" }]);
});

test("a message with nothing to read says nothing", () => {
  const blank = assistant({ type: "text", text: "  \n " }, { type: "text", text: `${ZERO_WIDTH_SPACE}` }, { type: "text" });
  expect(parseTimeline([blank], "claude")).toEqual([]);
});

test("the AI's thinking, images and unknown parts are not rows", () => {
  const line = assistant({ type: "thinking", thinking: "hm" }, { type: "image", source: {} }, { type: "something_new", text: "x" }, null);
  expect(parseTimeline([line], "claude")).toEqual([]);
});

test("results are matched to their calls by id, whatever order they come in", () => {
  const lines = [
    assistant(
      { type: "tool_use", id: "a", name: "Read", input: { file_path: "a.rs" } },
      { type: "tool_use", id: "b", name: "Bash", input: { command: "make" } },
    ),
    toolResults({ type: "tool_result", tool_use_id: "b", content: "built", is_error: false }),
    toolResults({ type: "tool_result", tool_use_id: "a", content: "contents of a", is_error: false }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "tool", text: "Read", detail: '{"file_path":"a.rs"}\n\ncontents of a' },
    { kind: "command", text: "make", detail: "built" },
  ]);
});

test("a result without a call it can be matched to joins the call before it, once", () => {
  const lines = [
    assistant({ type: "tool_use", id: "a", name: "Bash", input: { command: "make" } }),
    // No id at all, and an id nobody gave out: both mean "the call just made".
    toolResults({ type: "tool_result", content: "built" }),
    toolResults({ type: "tool_result", tool_use_id: "elsewhere", content: "a second answer" }),
  ];
  const entries = parseTimeline(lines, "claude");
  // The second one finds that call answered already and must not overwrite it: it is a row of its own.
  expect(entries).toEqual([
    { kind: "command", text: "make", detail: "built" },
    { kind: "tool", text: "tool_result", detail: "a second answer" },
  ]);
});

test("a result whose call is not in the log stands alone, and an empty one says nothing", () => {
  const lines = [
    toolResults({ type: "tool_result", tool_use_id: "gone", content: "something", is_error: true }),
    toolResults({ type: "tool_result", tool_use_id: "gone-too", content: "" }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([{ kind: "tool", text: "tool_result", detail: "something", failed: true }]);
});

test("a result can be text, a list of text parts or nothing; other parts are left out", () => {
  const lines = [
    assistant(
      { type: "tool_use", id: "a", name: "Bash", input: { command: "one" } },
      { type: "tool_use", id: "b", name: "Bash", input: { command: "two" } },
      { type: "tool_use", id: "c", name: "Bash", input: { command: "three" } },
    ),
    toolResults(
      { type: "tool_result", tool_use_id: "a", content: [{ type: "text", text: "first" }, { type: "image", source: {} }, { type: "text", text: "second" }] },
      { type: "tool_result", tool_use_id: "b", content: [] },
      { type: "tool_result", tool_use_id: "c" },
    ),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "command", text: "one", detail: "first\nsecond" },
    { kind: "command", text: "two" },
    { kind: "command", text: "three" },
  ]);
});

test("a command without output has no detail, but a failure is still marked", () => {
  const lines = [
    assistant({ type: "tool_use", id: "a", name: "Bash", input: { command: "false" } }),
    toolResults({ type: "tool_result", tool_use_id: "a", content: "", is_error: true }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([{ kind: "command", text: "false", failed: true }]);
});

test("a tool that fails is marked, with its input and what it said", () => {
  const lines = [
    assistant({ type: "tool_use", id: "a", name: "Edit", input: { file_path: "a.rs", old_string: "x" } }),
    toolResults({ type: "tool_result", tool_use_id: "a", content: "string not found", is_error: true }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "tool", text: "Edit", detail: '{"file_path":"a.rs","old_string":"x"}\n\nstring not found', failed: true },
  ]);
});

test("a tool call without input has only its result as detail", () => {
  const lines = [
    assistant({ type: "tool_use", id: "a", name: "ToolSearch" }),
    toolResults({ type: "tool_result", tool_use_id: "a", content: "found" }),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([{ kind: "tool", text: "ToolSearch", detail: "found" }]);
});

test("Bash without a command is shown as the tool it is", () => {
  const lines = [assistant({ type: "tool_use", id: "a", name: "Bash", input: {} }, { type: "tool_use", id: "b", name: "Bash", input: { command: "  " } })];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "tool", text: "Bash", detail: "{}" },
    { kind: "tool", text: "Bash", detail: '{"command":"  "}' },
  ]);
});

test("a multi-line command keeps its lines", () => {
  const command = "git commit -m 'feat: x\n\nwhy'";
  expect(parseTimeline([assistant({ type: "tool_use", id: "a", name: "Bash", input: { command } })], "claude")).toEqual([
    { kind: "command", text: command },
  ]);
});

test("names, commands, inputs and results from the executor are all cleaned", () => {
  const lines = [
    assistant(
      { type: "tool_use", id: "a", name: `Re${ESC}[2Jad`, input: { file_path: `a${RLO}.rs` } },
      { type: "tool_use", id: "b", name: "Bash", input: { command: `echo${ZERO_WIDTH_SPACE} ${ESC}[31mhi${ESC}[0m` } },
    ),
    toolResults(
      { type: "tool_result", tool_use_id: "a", content: `out${RLO}put` },
      { type: "tool_result", tool_use_id: "b", content: `${ESC}[32mhi${ESC}[0m\nsecond${ESC}]0;title${String.fromCharCode(7)} line` },
    ),
  ];
  expect(parseTimeline(lines, "claude")).toEqual([
    { kind: "tool", text: "Read", detail: '{"file_path":"a.rs"}\n\noutput' },
    { kind: "command", text: "echo hi", detail: "hi\nsecond line" },
  ]);
});

test("a tool's input is cut to 2000 characters, and so is its result", () => {
  const lines = [
    assistant({ type: "tool_use", id: "a", name: "Write", input: { content: "y".repeat(5000) } }),
    toolResults({ type: "tool_result", tool_use_id: "a", content: "z".repeat(5000) }),
  ];
  const [entry] = parseTimeline(lines, "claude");
  const [input, result] = entry.detail!.split("\n\n");
  expect(input).toHaveLength(2001);
  expect(input.endsWith("…")).toBe(true);
  expect(result).toBe(`${"z".repeat(2000)}…`);
});

test("the cut is counted after cleaning: 2000 readable characters stay whole", () => {
  // 2000 letters and an escape sequence that cleaning removes: the raw text is longer than 2000.
  const output = `${"a".repeat(1000)}${ESC}[31m${"b".repeat(1000)}${ESC}[0m`;
  const lines = [completed({ type: "command_execution", command: "x", aggregated_output: output, exit_code: 0 })];
  expect(parseTimeline(lines, "codex")[0].detail).toBe(`${"a".repeat(1000)}${"b".repeat(1000)}`);
  const longer = [completed({ type: "command_execution", command: "x", aggregated_output: `${output}c`, exit_code: 0 })];
  expect(parseTimeline(longer, "codex")[0].detail).toBe(`${"a".repeat(1000)}${"b".repeat(1000)}…`);
});

test("the cut never splits a character that takes two units", () => {
  // 1999 letters and a pair-of-units character: the cut would fall inside the pair.
  const grin = String.fromCodePoint(0x1f600);
  const lines = [completed({ type: "command_execution", command: "x", aggregated_output: `${"a".repeat(1999)}${grin}${grin}`, exit_code: 0 })];
  const detail = parseTimeline(lines, "codex")[0].detail!;
  expect(detail).toBe(`${"a".repeat(1999)}…`);
  expect(detail.length).toBeLessThanOrEqual(2001);
});

test("the end of an output loses its trailing blank space, the start keeps its indent", () => {
  const lines = [completed({ type: "command_execution", command: "x", aggregated_output: "  indented\nline\n\n  ", exit_code: 0 })];
  expect(parseTimeline(lines, "codex")[0].detail).toBe("  indented\nline");
});

test("the result of a run is its answer, and a failure is marked", () => {
  const ok = JSON.stringify({ type: "result", subtype: "success", is_error: false, result: "All done.\n\nSee the PR." });
  const bad = JSON.stringify({ type: "result", subtype: "success", is_error: true, result: "It broke" });
  expect(parseTimeline([ok, bad], "claude")).toEqual([
    { kind: "result", text: "All done.\n\nSee the PR." },
    { kind: "result", text: "It broke", failed: true },
  ]);
});

test("a result without a text is shown by its kind, and one without either is not shown", () => {
  const tooLong = JSON.stringify({ type: "result", subtype: "error_max_turns", is_error: true });
  const nothing = JSON.stringify({ type: "result", is_error: false });
  expect(parseTimeline([tooLong, nothing], "claude")).toEqual([{ kind: "result", text: "error_max_turns", failed: true }]);
});

test("Codex's answers, commands and tool calls are rows once they are completed, and not before", () => {
  const lines = [
    JSON.stringify({ type: "thread.started", thread_id: "t" }),
    JSON.stringify({ type: "turn.started" }),
    JSON.stringify({ type: "item.started", item: { type: "agent_message", text: "draft" } }),
    completed({ type: "agent_message", text: "I changed the file." }),
    completed({ type: "command_execution", command: "git status", aggregated_output: "clean\n", exit_code: 0, status: "completed" }),
    JSON.stringify({ type: "turn.completed", usage: { input_tokens: 1 } }),
  ];
  expect(parseTimeline(lines, "codex")).toEqual([
    { kind: "message", text: "I changed the file." },
    { kind: "command", text: "git status", detail: "clean" },
  ]);
});

test("the files Codex changed are a row of their own, each with what happened to it", () => {
  // The shape of Codex's file_change item (codex-rs exec_events.rs, read on 2026-10-14).
  const lines = [
    completed({
      type: "file_change",
      changes: [
        { path: "src/lib.rs", kind: "update" },
        { path: "docs/usage.md", kind: "add" },
        { path: "old.txt", kind: "delete" },
      ],
      status: "completed",
    }),
  ];
  expect(parseTimeline(lines, "codex")).toEqual([
    {
      kind: "files",
      text: "src/lib.rs\ndocs/usage.md\nold.txt",
      changes: [
        { kind: "update", path: "src/lib.rs" },
        { kind: "add", path: "docs/usage.md" },
        { kind: "delete", path: "old.txt" },
      ],
    },
  ]);
});

test("a change Codex could not apply is a failed row; its paths are cleaned (an escape sequence goes) and empty ones dropped", () => {
  const lines = [
    completed({
      type: "file_change",
      changes: [{ path: `src/a${ESC}[2J.rs`, kind: "update" }, { path: "  ", kind: "add" }, { path: "b.rs", kind: "renamed" }, "x"],
      status: "failed",
    }),
    // Nothing that names a file: no row.
    completed({ type: "file_change", changes: [], status: "completed" }),
    completed({ type: "file_change", status: "completed" }),
  ];
  expect(parseTimeline(lines, "codex")).toEqual([
    {
      kind: "files",
      text: "src/a.rs\nb.rs",
      changes: [
        { kind: "update", path: "src/a.rs" },
        // A kind the panel does not know is left unnamed.
        { kind: null, path: "b.rs" },
      ],
      failed: true,
    },
  ]);
});

test("an error Codex reports as an item is an error row, like its error events", () => {
  const lines = [completed({ type: "error", message: `model overloaded${ESC}[1m\ntrying again` }), completed({ type: "error", message: "" })];
  expect(parseTimeline(lines, "codex")).toEqual([{ kind: "error", text: "model overloaded\ntrying again" }]);
});

test("a command line longer than a tool's output may be is cut the same way", () => {
  const claude = [assistant({ type: "tool_use", id: "t1", name: "Bash", input: { command: "echo " + "x".repeat(5000) } })];
  const codex = [completed({ type: "command_execution", command: "y".repeat(5000), aggregated_output: "", exit_code: 0 })];
  for (const entry of [parseTimeline(claude, "claude")[0], parseTimeline(codex, "codex")[0]]) {
    expect(entry.kind).toBe("command");
    expect(entry.text).toHaveLength(2001);
    expect(entry.text.endsWith("…")).toBe(true);
  }
});

test("a Codex command fails by its exit code, or by its status when it has none", () => {
  const lines = [
    completed({ type: "command_execution", command: "a", aggregated_output: "", exit_code: 0, status: "failed" }),
    completed({ type: "command_execution", command: "b", aggregated_output: "", exit_code: 2 }),
    completed({ type: "command_execution", command: "c", aggregated_output: "", exit_code: null, status: "failed" }),
    completed({ type: "command_execution", command: "d", aggregated_output: "", exit_code: null, status: "completed" }),
    completed({ type: "command_execution", command: "e" }),
  ];
  expect(parseTimeline(lines, "codex")).toEqual([
    { kind: "command", text: "a" },
    { kind: "command", text: "b", failed: true },
    { kind: "command", text: "c", failed: true },
    { kind: "command", text: "d" },
    { kind: "command", text: "e" },
  ]);
});

test("a Codex tool call shows its arguments and what it returned, or why it failed", () => {
  const lines = [
    completed({
      type: "mcp_tool_call",
      server: "linear",
      tool: "get_issue",
      arguments: { id: "DEMO-2" },
      result: { content: [{ type: "text", text: "the issue" }], structured_content: null },
      error: null,
      status: "completed",
    }),
    // An error is a failure whatever the status says; a status of failed is one without an error.
    completed({
      type: "mcp_tool_call",
      server: "linear",
      tool: "save_comment",
      arguments: { issueId: "DEMO-2" },
      result: null,
      error: { message: "rate limited" },
      status: "completed",
    }),
    completed({ type: "mcp_tool_call", server: "linear", tool: "list_issues", arguments: {}, result: null, error: null, status: "failed" }),
    completed({ type: "mcp_tool_call", server: "linear", tool: "get_team", error: "plain text error" }),
    completed({ type: "mcp_tool_call", tool: "local_tool" }),
    completed({ type: "mcp_tool_call", server: "linear" }),
  ];
  expect(parseTimeline(lines, "codex")).toEqual([
    { kind: "tool", text: "linear.get_issue", detail: '{"id":"DEMO-2"}\n\nthe issue' },
    { kind: "tool", text: "linear.save_comment", detail: '{"issueId":"DEMO-2"}\n\nrate limited', failed: true },
    { kind: "tool", text: "linear.list_issues", detail: "{}", failed: true },
    { kind: "tool", text: "linear.get_team", detail: "plain text error", failed: true },
    { kind: "tool", text: "local_tool" },
  ]);
});

test("Codex's errors and a failed turn are error rows, cleaned", () => {
  const lines = [
    JSON.stringify({ type: "error", message: `Reconnecting${ESC}[2K... 2/5` }),
    JSON.stringify({ type: "turn.failed", error: { message: "stream closed" } }),
    JSON.stringify({ type: "turn.failed" }),
    JSON.stringify({ type: "error" }),
  ];
  expect(parseTimeline(lines, "codex")).toEqual([
    { kind: "error", text: "Reconnecting... 2/5" },
    { kind: "error", text: "stream closed" },
    { kind: "error", text: "turn.failed" },
  ]);
});

test("events that say nothing a reader needs are left out", () => {
  const skipped = [
    JSON.stringify({ type: "rate_limit_event", rate_limit_info: {} }),
    JSON.stringify({ type: "system", subtype: "hook_started" }),
    JSON.stringify({ type: "user", message: { content: "plain text, not a result" } }),
    JSON.stringify({ type: "assistant" }),
    JSON.stringify({ type: "assistant", message: { content: "not a list" } }),
    JSON.stringify({ thread: "no type, no stream" }),
    JSON.stringify({ stream: "stderr" }),
    JSON.stringify({ stream: "stderr", text: "   " }),
    "",
    "   ",
  ];
  expect(parseTimeline(skipped, null)).toEqual([]);
});

test("a line that is JSON but not an event is shown as the unreadable line it is", () => {
  expect(parseTimeline(["42", '"just text"', "null", "[1,2]", "{broken"], "claude")).toEqual([
    { kind: "stderr", text: "42" },
    { kind: "stderr", text: '"just text"' },
    { kind: "stderr", text: "null" },
    { kind: "stderr", text: "[1,2]" },
    { kind: "stderr", text: "{broken" },
  ]);
});

test("a line of another stream is still shown", () => {
  expect(parseTimeline([JSON.stringify({ stream: "host", text: "something new" })], "claude")).toEqual([
    { kind: "stderr", text: "something new" },
  ]);
});

test("an executor's events are read in its own dialect only; without a known executor, in both", () => {
  const claude = assistant({ type: "text", text: "from Claude" });
  const codex = completed({ type: "agent_message", text: "from Codex" });
  expect(parseTimeline([codex, claude], "claude")).toEqual([{ kind: "message", text: "from Claude" }]);
  expect(parseTimeline([codex, claude], "codex")).toEqual([{ kind: "message", text: "from Codex" }]);
  expect(parseTimeline([codex, claude], null)).toEqual([
    { kind: "message", text: "from Codex" },
    { kind: "message", text: "from Claude" },
  ]);
});

test("IssueRelay's own lines are read the same whatever the executor", () => {
  const lines = [JSON.stringify({ stream: "runner", text: "spawn failed" }), JSON.stringify({ stream: "stderr", text: "oops" })];
  const expected = [
    { kind: "note", text: "spawn failed" },
    { kind: "stderr", text: "oops" },
  ];
  for (const executor of ["claude", "codex", null] as const) {
    expect(parseTimeline(lines, executor)).toEqual(expected);
  }
});

test("a log with no lines has no rows, and the lines given are not changed", () => {
  expect(parseTimeline([], "claude")).toEqual([]);
  const lines = [assistant({ type: "text", text: "hi" })];
  const copy = [...lines];
  parseTimeline(lines, "claude");
  expect(lines).toEqual(copy);
});

test("parsing text that tries to be markup or a script leaves it as text", () => {
  const text = '<img src=x onerror="alert(1)"> <script>alert(2)</script> **bold**';
  expect(parseTimeline([assistant({ type: "text", text })], "claude")).toEqual([{ kind: "message", text }]);
});

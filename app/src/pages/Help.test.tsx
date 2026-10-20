import { screen, within } from "@testing-library/react";
import { vi } from "vitest";
import en from "@/i18n/locales/en.json";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { JOB_STATES } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import Help from "./Help";

test("the help page states the trigger conditions in full", async () => {
  await renderWithProviders(<Help />, { lang: "zh-CN" });
  // The page repeats ai-ready and Todo in its examples and FAQ, so the four conditions are read
  // from their own list.
  const conditions = await screen.findByRole("region", { name: "触发条件" });
  const items = within(conditions).getAllByRole("listitem");
  expect(items).toHaveLength(4);
  expect(items[0]).toHaveTextContent("指派给自己");
  expect(items[1]).toHaveTextContent("ai-ready");
  expect(items[2]).toHaveTextContent("Todo");
  expect(items[3]).toHaveTextContent("已绑定");
});

test("without the settings the page still helps, with the default names", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = vi.fn().mockRejectedValue(new BackendError("IssueRelay could not start"));
  await renderWithProviders(<Help />, { backend });
  expect(await screen.findByText("Showing the default names: the settings could not be read.")).toBeInTheDocument();
  const conditions = screen.getByRole("region", { name: "Trigger conditions" });
  expect(within(conditions).getAllByRole("listitem")[1]).toHaveTextContent("ai-ready");
});

// ---- helpers ----------------------------------------------------------------------------------

/** The demo backend whose trigger names are the ones given. */
async function backendNaming(trigger: Partial<Record<"label" | "todo" | "in_progress" | "in_testing" | "done", string>>) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const settings = await backend.settings();
  backend.settings = vi.fn().mockResolvedValue({ ...settings, trigger: { ...settings.trigger, ...trigger } });
  return backend;
}

// ---- the page ---------------------------------------------------------------------------------

test("the page is five sections, each named by its heading", async () => {
  await renderWithProviders(<Help />);
  expect(await screen.findByRole("heading", { level: 1, name: "Help" })).toBeInTheDocument();
  const names = ["Trigger conditions", "How an issue moves", "Writing an issue", "Lean runs", "Questions and answers"];
  expect(screen.getAllByRole("heading", { level: 2 }).map((heading) => heading.textContent)).toEqual(names);
  for (const name of names) expect(screen.getByRole("region", { name })).toBeInTheDocument();
});

test("the four conditions are said in English, with the names of the settings in them", async () => {
  const backend = await backendNaming({ label: "ready-for-ai", todo: "Backlog" });
  await renderWithProviders(<Help />, { backend });
  const conditions = await screen.findByRole("region", { name: "Trigger conditions" });
  // The names come with the settings: until they are read the defaults stand.
  await within(conditions).findByText("It has the ready-for-ai label");
  const items = within(conditions).getAllByRole("listitem");
  expect(items.map((item) => item.textContent)).toEqual([
    "The issue is assigned to you",
    "It has the ready-for-ai label",
    "Its status is Backlog",
    "Its project is bound to a local folder and the executor is ready",
  ]);
});

test("while the settings are read the default names stand, and there is no note about them", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const settings = await backend.settings();
  let release: ((value: typeof settings) => void) | undefined;
  backend.settings = vi.fn(() => new Promise<typeof settings>((resolve) => (release = resolve)));
  await renderWithProviders(<Help />, { backend });
  const conditions = screen.getByRole("region", { name: "Trigger conditions" });
  expect(within(conditions).getAllByRole("listitem")[1]).toHaveTextContent("It has the ai-ready label");
  expect(screen.queryByText(/Showing the default names/)).not.toBeInTheDocument();
  release?.({ ...settings, trigger: { ...settings.trigger, label: "ready-for-ai" } });
  await within(conditions).findByText("It has the ready-for-ai label");
});

test("with the settings read there is no note about default names", async () => {
  await renderWithProviders(<Help />);
  await screen.findByRole("region", { name: "Trigger conditions" });
  expect(screen.queryByText(/Showing the default names/)).not.toBeInTheDocument();
});

test("the way an issue moves names the statuses of the settings and the one change IssueRelay makes in Linear", async () => {
  const backend = await backendNaming({ todo: "Backlog", in_progress: "Doing", in_testing: "Review", done: "Shipped" });
  await renderWithProviders(<Help />, { backend });
  const flow = await screen.findByRole("region", { name: "How an issue moves" });
  const steps = await within(flow).findByRole("list", { name: "The statuses of an issue in Linear" });
  await within(steps).findByText("Shipped");
  expect(within(steps).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    "Backlog",
    "Doing",
    "Review",
    "Marked done in the panel",
    "Shipped",
  ]);
  expect(within(flow).getByText(/the only change IssueRelay makes in Linear/)).toBeInTheDocument();
});

test("the default names are the ones the flow shows without settings", async () => {
  await renderWithProviders(<Help />);
  const steps = await screen.findByRole("list", { name: "The statuses of an issue in Linear" });
  expect(within(steps).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    "Todo",
    "In Progress",
    "In Testing",
    "Marked done in the panel",
    "Done",
  ]);
});

test("the eight local statuses each have their badge and what it means", async () => {
  await renderWithProviders(<Help />);
  const legend = await screen.findByRole("list", { name: "What the statuses in the panel mean" });
  const items = within(legend).getAllByRole("listitem");
  expect(items).toHaveLength(JOB_STATES.length);
  const badges = ["Waiting for setup", "Queued", "Running", "Awaiting acceptance", "Needs information", "Done", "Failed", "Canceled"];
  JOB_STATES.forEach((_state, index) => {
    // The badge is the state's own, the sentence next to it says what it is.
    expect(items[index]).toHaveTextContent(badges[index]);
    expect(within(items[index]).getAllByText(/.+/).length).toBeGreaterThan(1);
  });
  expect(items[0]).toHaveTextContent(/not bound|bound/);
});

test("a clear issue and a vague one are shown side by side with the reason", async () => {
  await renderWithProviders(<Help />);
  const examples = await screen.findByRole("region", { name: "Writing an issue" });
  const good = within(examples).getByRole("group", { name: "A request the AI can finish" });
  expect(within(good).getByText(en.help.example.good_title)).toBeInTheDocument();
  expect(within(good).getByText(en.help.example.good_body)).toBeInTheDocument();
  const vague = within(examples).getByRole("group", { name: "A request the AI will ask about" });
  expect(within(vague).getByText(en.help.example.vague_title)).toBeInTheDocument();
  expect(within(vague).getByText(en.help.example.vague_body)).toBeInTheDocument();
  expect(within(vague).getByText(/sets the status back to Todo and removes the label/)).toBeInTheDocument();
});

test("a vague issue is said to go back to the status the settings name", async () => {
  const backend = await backendNaming({ todo: "Backlog" });
  await renderWithProviders(<Help />, { backend });
  const examples = await screen.findByRole("region", { name: "Writing an issue" });
  const vague = within(examples).getByRole("group", { name: "A request the AI will ask about" });
  await within(vague).findByText(/sets the status back to Backlog and removes the label/);
});

// The first-run guide shows the clear example next to the four conditions and finds its step by
// the words "assigned to you": the example must not use them.
test("the example issues do not repeat the condition that the guide finds its last step by", () => {
  for (const key of ["good_title", "good_body", "vague_title", "vague_body"] as const) {
    expect(en.help.example[key], key).not.toMatch(/assigned to you/);
  }
});

test("every language has the four example texts the guide reads", async () => {
  const files = await Promise.all(["zh-CN", "zh-TW", "ja", "ko"].map((lang) => import(`@/i18n/locales/${lang}.json`)));
  for (const file of [en, ...files.map((module) => module.default)]) {
    for (const key of ["good_title", "good_body", "vague_title", "vague_body"] as const) {
      expect(file.help.example[key].trim(), key).not.toBe("");
    }
  }
});

// ---- the questions ----------------------------------------------------------------------------

test("the questions are the five the design names, each followed by its answer", async () => {
  await renderWithProviders(<Help />);
  const faq = await screen.findByRole("region", { name: "Questions and answers" });
  expect(within(faq).getAllByRole("heading", { level: 3 }).map((heading) => heading.textContent)).toEqual([
    "Why was my issue not picked up?",
    "What do the three permission levels mean?",
    "What if I need a proxy?",
    "How can Claude Code or Codex use IssueRelay's MCP control server?",
    "Where are the logs?",
  ]);
});

test("the answer to why an issue is not picked up goes through the four conditions with the names of the settings", async () => {
  const backend = await backendNaming({ label: "ready-for-ai", todo: "Backlog" });
  await renderWithProviders(<Help />, { backend });
  const faq = await screen.findByRole("region", { name: "Questions and answers" });
  const answer = await within(faq).findByText(/ready-for-ai/);
  expect(answer).toHaveTextContent("Backlog");
  expect(answer).toHaveTextContent("The Issues page says why for each issue");
  expect(answer).toHaveTextContent("Run again");
});

test("the answer about permissions has the three levels and what each means", async () => {
  await renderWithProviders(<Help />);
  const faq = await screen.findByRole("region", { name: "Questions and answers" });
  const levels = within(faq).getByRole("list", { name: "The three permission levels" });
  const items = within(levels).getAllByRole("listitem");
  expect(items).toHaveLength(3);
  expect(items[0]).toHaveTextContent("Full access");
  expect(items[0]).toHaveTextContent("never asked to confirm anything");
  expect(items[1]).toHaveTextContent("Automatic review");
  expect(items[2]).toHaveTextContent("Personal settings");
  expect(items[2]).toHaveTextContent("Codex follows ~/.codex/config.toml");
});

test("the answer about a proxy says where the variables are read from and where to look at them", async () => {
  await renderWithProviders(<Help />);
  const faq = await screen.findByRole("region", { name: "Questions and answers" });
  const answer = within(faq).getByText(/HTTPS_PROXY/);
  expect(answer).toHaveTextContent("login shell");
  expect(answer).toHaveTextContent("Environment");
});

test("the answer about the control server says it is registered for the user, that unattended runs that are not lean load it, and that its tools can pause", async () => {
  await renderWithProviders(<Help />);
  const faq = await screen.findByRole("region", { name: "Questions and answers" });
  const answer = within(faq).getByText(/registered for your user/);
  expect(answer).toHaveTextContent("unattended runs that are not lean load it too");
  expect(answer).toHaveTextContent("pause");
  expect(answer).toHaveTextContent("cannot mark an issue done");
});

test("the answer about the logs names the run's own log and where the files are on each system", async () => {
  await renderWithProviders(<Help />);
  const faq = await screen.findByRole("region", { name: "Questions and answers" });
  const answer = within(faq).getByText(/Raw log/);
  expect(answer).toHaveTextContent("com.sunkailun.issuerelay");
  expect(answer).toHaveTextContent("logs");
});

// ---- languages --------------------------------------------------------------------------------

test.each([
  ["zh-CN", "触发条件", "怎样写需求"],
  ["zh-TW", "觸發條件", "怎麼寫需求"],
  ["ja", "トリガー条件", "課題の書き方"],
  ["ko", "트리거 조건", "이슈 작성법"],
] as const)("the page reads in %s", async (lang, conditions, examples) => {
  await renderWithProviders(<Help />, { lang });
  expect(await screen.findByRole("region", { name: conditions })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: examples })).toBeInTheDocument();
  expect(within(screen.getByRole("region", { name: conditions })).getAllByRole("listitem")).toHaveLength(4);
});

// ---- lean runs --------------------------------------------------------------------------------

test("the lean runs section says what they leave out, when to turn them off and where the effort is set", async () => {
  await renderWithProviders(<Help />);
  const lean = await screen.findByRole("region", { name: "Lean runs" });
  expect(lean).toHaveTextContent("only Linear's MCP server and the repository's own rules and settings");
  // What Codex still reads is said, as the design asks.
  expect(lean).toHaveTextContent("~/.codex/AGENTS.md");
  expect(lean).toHaveTextContent("Turn it off for a binding that needs other MCP servers");
  expect(lean).toHaveTextContent(".codex/");
  expect(lean).toHaveTextContent("deny rules");
  expect(lean).toHaveTextContent("custom model provider");
  expect(lean).toHaveTextContent("reasoning effort");
});

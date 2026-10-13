import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { WorkspaceReport } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { WorkspaceCheck } from "./WorkspaceCheck";

const COMPLETE: WorkspaceReport = { label_ok: true, missing_states: [], teams_checked: ["Demo"] };

test("a complete workspace is two passed checks and the teams that were looked at", async () => {
  await renderWithProviders(<WorkspaceCheck report={COMPLETE} labelName="ai-ready" />);
  expect(screen.getByText("The label ai-ready exists")).toBeInTheDocument();
  expect(screen.getByText("The statuses IssueRelay uses exist")).toBeInTheDocument();
  expect(screen.getByText("Teams checked: Demo")).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

test("each check has a symbol in the colour of its result", async () => {
  await renderWithProviders(<WorkspaceCheck report={{ label_ok: false, missing_states: [], teams_checked: [] }} labelName="ai-ready" />);
  const symbol = (text: string) => screen.getByText(text).closest("li")?.querySelector("svg")?.getAttribute("class") ?? "";
  expect(symbol("The label ai-ready is missing")).toContain("text-state-failed");
  expect(symbol("The statuses IssueRelay uses exist")).toContain("text-state-done");
});

test("the label named is the one the caller gives", async () => {
  await renderWithProviders(<WorkspaceCheck report={COMPLETE} labelName="ready-for-ai" />);
  expect(screen.getByText("The label ready-for-ai exists")).toBeInTheDocument();
});

// The guide's workspace step looks for the label with `findAllByText`: the check and the way to
// add it both name it.
test("a missing label is named in the check and in how to add it", async () => {
  await renderWithProviders(<WorkspaceCheck report={{ ...COMPLETE, label_ok: false }} labelName="ai-ready" />);
  expect(screen.getByText("The label ai-ready is missing")).toBeInTheDocument();
  expect(screen.getAllByText(/ai-ready/)).toHaveLength(2);
  expect(screen.getByText(/Create the label ai-ready in Linear \(Settings → Labels\)/)).toBeInTheDocument();
});

test("missing statuses are listed, and where to add them is said", async () => {
  const report: WorkspaceReport = { label_ok: true, missing_states: ["In Progress", "Done"], teams_checked: ["Demo", "Docs"] };
  await renderWithProviders(<WorkspaceCheck report={report} labelName="ai-ready" />);
  expect(screen.getByText("Missing statuses: In Progress, Done")).toBeInTheDocument();
  expect(screen.getByText(/Add the missing statuses in Linear under Settings → Teams → Workflow/)).toBeInTheDocument();
  expect(screen.getByText("Teams checked: Demo, Docs")).toBeInTheDocument();
});

test("the button opens Linear's home page, which is as far as the program goes", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const open = vi.spyOn(backend, "openLinear");
  await renderWithProviders(<WorkspaceCheck report={{ ...COMPLETE, missing_states: ["Done"] }} labelName="ai-ready" />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Open Linear" }));
  expect(open).toHaveBeenCalledTimes(1);
  expect(open).toHaveBeenCalledWith("https://linear.app/");
});

test("a Linear that cannot be opened is told, not left clicking", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.openLinear = vi.fn().mockRejectedValue(new BackendError("the system has no browser"));
  await renderWithProviders(<WorkspaceCheck report={{ ...COMPLETE, label_ok: false }} labelName="ai-ready" />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Open Linear" }));
  expect(await screen.findByText("Could not open Linear")).toBeInTheDocument();
  expect(screen.getByText("the system has no browser")).toBeInTheDocument();
});

test("no teams are named when none was looked at", async () => {
  await renderWithProviders(<WorkspaceCheck report={{ ...COMPLETE, teams_checked: [] }} labelName="ai-ready" />);
  expect(screen.queryByText(/Teams checked/)).not.toBeInTheDocument();
});

test("the checks are a list the card names", async () => {
  await renderWithProviders(<WorkspaceCheck report={COMPLETE} labelName="ai-ready" />);
  const items = screen.getAllByRole("listitem");
  expect(items).toHaveLength(2);
  expect(within(items[0]).getByText("The label ai-ready exists")).toBeInTheDocument();
});

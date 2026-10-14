import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { IssueLink } from "./IssueLink";

const URL = "https://linear.app/demo/issue/DEMO-4";

test("the identifier is a button named for what it does, and it opens the issue through the backend", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const open = vi.spyOn(backend, "openLinear");
  await renderWithProviders(<IssueLink identifier="DEMO-4" url={URL} />, { backend });
  const button = screen.getByRole("button", { name: "Open DEMO-4 in Linear" });
  // What is read on screen is the identifier; the tooltip says what the click does.
  expect(button).toHaveTextContent(/^DEMO-4$/);
  expect(button).toHaveAttribute("title", "Open in Linear");
  expect(button).toHaveAttribute("type", "button");
  await userEvent.click(button);
  expect(open).toHaveBeenCalledTimes(1);
  expect(open).toHaveBeenCalledWith(URL);
});

test("the keyboard opens it too", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const open = vi.spyOn(backend, "openLinear");
  await renderWithProviders(<IssueLink identifier="DEMO-4" url={URL} />, { backend });
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "Open DEMO-4 in Linear" })).toHaveFocus();
  await userEvent.keyboard("{Enter}");
  expect(open).toHaveBeenCalledWith(URL);
});

test("a link the app refuses to open is reported with the translated reason", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.openLinear = vi.fn().mockRejectedValue(new BackendError("not_a_linear_link"));
  await renderWithProviders(<IssueLink identifier="DEMO-4" url="https://example.com/not-linear" />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Open DEMO-4 in Linear" }));
  expect(await screen.findByText("IssueRelay only opens Linear's own pages")).toBeInTheDocument();
});

test("a link that cannot be opened for another reason says so, with the reason", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.openLinear = vi.fn().mockRejectedValue(new BackendError("no browser is available"));
  await renderWithProviders(<IssueLink identifier="DEMO-4" url={URL} />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Open DEMO-4 in Linear" }));
  expect(await screen.findByText("Could not open Linear")).toBeInTheDocument();
  expect(screen.getByText("no browser is available")).toBeInTheDocument();
});

test("the name and the tooltip are in the interface language", async () => {
  await renderWithProviders(<IssueLink identifier="DEMO-4" url={URL} />, { lang: "zh-CN" });
  const button = screen.getByRole("button", { name: "在 Linear 中打开 DEMO-4" });
  expect(button).toHaveAttribute("title", "在 Linear 中打开");
});

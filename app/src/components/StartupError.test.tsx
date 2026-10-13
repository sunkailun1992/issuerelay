import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { StartupError } from "./StartupError";

test("a second instance explains itself and can quit", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.requestQuit = vi.fn().mockResolvedValue(undefined);
  await renderWithProviders(
    <StartupError code="already_running" detail="another IssueRelay instance is already running" />,
    { backend },
  );
  expect(screen.getByText(/Another IssueRelay is already running/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Quit" }));
  expect(backend.requestQuit).toHaveBeenCalled();
});

test.each([
  ["store_unavailable", "IssueRelay could not open its run records"],
  ["other", "Something stopped IssueRelay from starting"],
] as const)("a %s failure has a translated headline, the cleaned detail and Quit", async (code, headline) => {
  await renderWithProviders(<StartupError code={code} detail={"cannot open state.db: disk full\u001b[2J"} />);
  expect(screen.getByText(headline)).toBeInTheDocument();
  expect(screen.getByText("cannot open state.db: disk full")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Quit" })).toBeInTheDocument();
});

test("the page is titled, in the interface language", async () => {
  await renderWithProviders(<StartupError code="other" detail="boom" />, { lang: "zh-CN" });
  expect(screen.getByRole("heading", { name: "IssueRelay 无法启动" })).toBeInTheDocument();
  expect(screen.getByText("出现问题，IssueRelay 无法启动")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "退出" })).toBeInTheDocument();
});

test("a quit the app does not take is reported, not swallowed", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.requestQuit = vi.fn().mockRejectedValue(new BackendError("the window is gone"));
  await renderWithProviders(<StartupError code="other" detail="boom" />, { backend });
  await userEvent.click(screen.getByRole("button", { name: "Quit" }));
  expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
  expect(screen.getByText("the window is gone")).toBeInTheDocument();
});

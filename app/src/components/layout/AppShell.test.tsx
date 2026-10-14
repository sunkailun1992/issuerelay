import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes } from "react-router";
import { vi } from "vitest";
import { queryKeys } from "@/hooks/queries";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { StatusView, UpdateStatus } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import iconSvg from "../../../public/icon.svg?raw";
import { AppShell } from "./AppShell";

test("the sidebar lists every page and shows the monitoring state", async () => {
  await renderWithProviders(<AppShell />, { lang: "zh-CN" });
  for (const name of ["概览", "需求", "运行记录", "项目绑定", "环境检查", "设置", "使用说明"]) {
    expect(screen.getByRole("link", { name })).toBeInTheDocument();
  }
  expect(await screen.findByText("监控中")).toBeInTheDocument();
});

test("the top of the sidebar shows the app icon, a file the panel ships", async () => {
  const { container } = await renderWithProviders(<AppShell />);
  // Decorative: the name beside it says what it is. `/icon.svg` is app/public/icon.svg, which the
  // build copies as it is and `pnpm --dir app icons --option <x>` writes; the import above fails
  // the whole file if it is missing.
  const icon = container.querySelector("img");
  expect(icon).toHaveAttribute("src", "/icon.svg");
  expect(icon).toHaveAttribute("alt", "");
  expect(iconSvg).toMatch(/^<svg\b/);
});

test("each link goes to its page", async () => {
  await renderWithProviders(<AppShell />);
  const targets = {
    Overview: "/",
    Issues: "/issues",
    Runs: "/runs",
    Bindings: "/bindings",
    Environment: "/doctor",
    Settings: "/settings",
    Help: "/help",
  };
  for (const [name, path] of Object.entries(targets)) {
    expect(screen.getByRole("link", { name }), name).toHaveAttribute("href", path);
  }
});

test("the page being shown is marked, a run's page counting as part of the runs", async () => {
  await renderWithProviders(<AppShell />, { route: "/runs/12" });
  expect(screen.getByRole("link", { name: "Runs" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("link", { name: "Overview" })).not.toHaveAttribute("aria-current");
});

test("the overview is marked only on the overview itself", async () => {
  await renderWithProviders(<AppShell />, { route: "/" });
  expect(screen.getByRole("link", { name: "Overview" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("link", { name: "Runs" })).not.toHaveAttribute("aria-current");
});

test("the foot of the sidebar shows the version", async () => {
  await renderWithProviders(<AppShell />);
  expect(await screen.findByText("Version 0.1.0")).toBeInTheDocument();
});

test("a paused monitor says so", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await backend.pause();
  await renderWithProviders(<AppShell />, { backend });
  expect(await screen.findByText("Paused")).toBeInTheDocument();
});

test("a start whose first connection is under way says so, and is no problem yet", async () => {
  await renderWithProviders(<AppShell />, { backend: createMockBackend("connecting", { latencyMs: 0 }) });
  expect(await screen.findByText("Connecting to Linear…")).toBeInTheDocument();
  expect(screen.queryByText("Needs attention")).not.toBeInTheDocument();
});

test("a problem is named and marked in the failure colour", async () => {
  await renderWithProviders(<AppShell />, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText("Needs attention")).toHaveClass("text-state-failed");
});

test("waiting for runs to finish comes before everything else", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  const view = await backend.status();
  backend.status = async (): Promise<StatusView> => ({ ...view, status: { ...view.status, draining: true } });
  await renderWithProviders(<AppShell />, { backend });
  expect(await screen.findByText("Waiting for runs to finish")).toBeInTheDocument();
  expect(screen.queryByText("Needs attention")).not.toBeInTheDocument();
});

test("when the status cannot be read only the version shows", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.status = async () => {
    throw new BackendError("the engine did not answer");
  };
  await renderWithProviders(<AppShell />, { backend });
  expect(await screen.findByText("Version 0.1.0")).toBeInTheDocument();
  for (const text of ["Monitoring", "Paused", "Needs attention", "Connecting to Linear…", "Waiting for runs to finish"]) {
    expect(screen.queryByText(text), text).not.toBeInTheDocument();
  }
});

test("following a link moves the highlight", async () => {
  await renderWithProviders(<AppShell />);
  await userEvent.click(screen.getByRole("link", { name: "Settings" }));
  expect(screen.getByRole("link", { name: "Settings" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("link", { name: "Overview" })).not.toHaveAttribute("aria-current");
});

test("a new version shows its banner above the page, and nothing is shown while up to date", async () => {
  const page = (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<h1>Demo page</h1>} />
      </Route>
    </Routes>
  );
  const quiet = await renderWithProviders(page);
  await waitFor(() => expect(quiet.client.getQueryState(queryKeys.update)?.status).toBe("success"));
  expect(screen.getByRole("heading", { name: "Demo page" })).toBeInTheDocument();
  expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
  quiet.unmount();
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async (): Promise<UpdateStatus> => ({ kind: "available", version: "0.2.0" });
  await renderWithProviders(page, { backend });
  const banner = await screen.findByText("IssueRelay 0.2.0 is available");
  const heading = screen.getByRole("heading", { name: "Demo page" });
  expect(banner.compareDocumentPosition(heading) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test("a banner that cannot be drawn shows nothing, and the shell and the page stay", async () => {
  // React reports an error it hands to a boundary; keep the output of the test run readable.
  const error = vi.spyOn(console, "error").mockImplementation(() => {});
  const backend = createMockBackend("demo", { latencyMs: 0 });
  // A reply the banner cannot make sense of: a reason that is no text.
  backend.updateStatus = async () => ({ kind: "failed", code: "install_failed", detail: 5 }) as unknown as UpdateStatus;
  const { client } = await renderWithProviders(
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<h1>Demo page</h1>} />
      </Route>
    </Routes>,
    { backend },
  );
  await waitFor(() => expect(client.getQueryState(queryKeys.update)?.status).toBe("success"));
  await waitFor(() => expect(error.mock.calls.map(([message]) => message)).toContain("a part of the panel stopped on an error"));
  expect(screen.getByRole("heading", { name: "Demo page" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Settings" })).toBeInTheDocument();
  expect(screen.queryByText("The update could not be installed")).not.toBeInTheDocument();
});

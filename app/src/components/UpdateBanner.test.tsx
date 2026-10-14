import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { EngineEventsBridge } from "@/components/EngineEventsBridge";
import { queryKeys } from "@/hooks/queries";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { UpdateStatus } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { UpdateBanner } from "./UpdateBanner";

const AVAILABLE: UpdateStatus = { kind: "available", version: "0.2.0" };

test("a newer version shows the banner, and installing is the backend's job", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  const install = vi.spyOn(backend, "installUpdate");
  const quit = vi.spyOn(backend, "quit");
  const requestQuit = vi.spyOn(backend, "requestQuit");
  await renderWithProviders(<UpdateBanner />, { backend });
  expect(await screen.findByText("IssueRelay 0.2.0 is available")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Install and restart" }));
  expect(install).toHaveBeenCalledTimes(1);
  // The panel never quits or restarts by itself: the Rust side asks about runs first.
  expect(quit).not.toHaveBeenCalled();
  expect(requestQuit).not.toHaveBeenCalled();
});

test("the offer is a status for a screen reader, and says that installing restarts the app", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  await renderWithProviders(<UpdateBanner />, { backend });
  const banner = await screen.findByRole("status");
  expect(banner).toHaveTextContent("IssueRelay 0.2.0 is available");
  expect(banner).toHaveTextContent("IssueRelay restarts to install it; if runs are in progress you are asked first.");
});

test("while the update downloads the button says so and cannot be pressed again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  backend.installUpdate = vi.fn(() => new Promise<void>(() => {}));
  await renderWithProviders(<UpdateBanner />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Install and restart" }));
  expect(await screen.findByRole("button", { name: "Downloading and verifying…" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Downloading and verifying…" }));
  expect(backend.installUpdate).toHaveBeenCalledTimes(1);
});

test("a failed install says why in the user's language and keeps the banner", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  backend.installUpdate = vi.fn().mockRejectedValue(new BackendError("update_bad_signature"));
  await renderWithProviders(<UpdateBanner />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Install and restart" }));
  expect(await screen.findByText("The update's signature did not verify; nothing was installed")).toBeInTheDocument();
  expect(screen.getByText("IssueRelay 0.2.0 is available")).toBeInTheDocument();
  // The button is there again for another try.
  expect(screen.getByRole("button", { name: "Install and restart" })).toBeEnabled();
});

test("no banner while up to date; Later hides it for that version", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const { client, unmount } = await renderWithProviders(<UpdateBanner />, { backend });
  // The answer (up to date) has arrived, so the absence below is a real one.
  await waitFor(() => expect(client.getQueryState(queryKeys.update)?.status).toBe("success"));
  expect(screen.queryByRole("button", { name: "Install and restart" })).not.toBeInTheDocument();
  unmount();
  backend.updateStatus = async () => AVAILABLE;
  await renderWithProviders(<UpdateBanner />, { backend });
  await userEvent.click(await screen.findByRole("button", { name: "Later" }));
  expect(screen.queryByText("IssueRelay 0.2.0 is available")).not.toBeInTheDocument();
});

test("Later is for that version only: a newer one shows the banner again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  let announce: (status: UpdateStatus) => void = () => {};
  backend.onUpdateStatus = async (listener) => {
    announce = listener;
    return () => {};
  };
  await renderWithProviders(
    <>
      <EngineEventsBridge />
      <UpdateBanner />
    </>,
    { backend },
  );
  await userEvent.click(await screen.findByRole("button", { name: "Later" }));
  expect(screen.queryByText("IssueRelay 0.2.0 is available")).not.toBeInTheDocument();
  await act(async () => announce({ kind: "available", version: "0.3.0" }));
  expect(await screen.findByText("IssueRelay 0.3.0 is available")).toBeInTheDocument();
});

test("a background check that finds a version shows the banner in an open panel", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let announce: (status: UpdateStatus) => void = () => {};
  const listen = vi.fn(async (listener: (status: UpdateStatus) => void) => {
    announce = listener;
    return () => {};
  });
  backend.onUpdateStatus = listen;
  const { client } = await renderWithProviders(
    <>
      <EngineEventsBridge />
      <UpdateBanner />
    </>,
    { backend },
  );
  await waitFor(() => expect(listen).toHaveBeenCalled());
  // The first read (up to date) has settled, so it cannot overwrite the event afterwards.
  await waitFor(() => expect(client.getQueryState(queryKeys.update)?.status).toBe("success"));
  await act(async () => announce(AVAILABLE));
  expect(await screen.findByText("IssueRelay 0.2.0 is available")).toBeInTheDocument();
});

test("an update that could not be installed before the restart is shown once, with why", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async (): Promise<UpdateStatus> => ({ kind: "failed", code: "install_failed", detail: "cannot replace the app" });
  await renderWithProviders(<UpdateBanner />, { backend });
  expect(await screen.findByText("The update could not be installed")).toBeInTheDocument();
  expect(screen.getByText("cannot replace the app")).toBeInTheDocument();
  expect(screen.getByRole("alert")).toHaveTextContent("The update could not be installed");
  await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(screen.queryByText("The update could not be installed")).not.toBeInTheDocument();
});

test("the reason of a failed install is the engine's text, cleaned, under the translated headline", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async (): Promise<UpdateStatus> => ({
    kind: "failed",
    code: "install_failed",
    detail: "cannot replace the app\u001b[31m: permission denied",
  });
  await renderWithProviders(<UpdateBanner />, { backend });
  expect(await screen.findByText("The update could not be installed")).toBeInTheDocument();
  expect(screen.getByText("cannot replace the app: permission denied")).toBeInTheDocument();
});

test("a failed background check stays out of the banner", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async (): Promise<UpdateStatus> => ({ kind: "failed", code: "unreachable", detail: "offline" });
  const { client } = await renderWithProviders(<UpdateBanner />, { backend });
  await waitFor(() => expect(client.getQueryState(queryKeys.update)?.status).toBe("success"));
  expect(screen.queryByText("Could not reach the update server")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Install and restart" })).not.toBeInTheDocument();
});

test("the banner reads in the language of the panel", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.updateStatus = async () => AVAILABLE;
  await renderWithProviders(<UpdateBanner />, { backend, lang: "zh-CN" });
  expect(await screen.findByText("IssueRelay 0.2.0 已发布")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "安装并重启" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "稍后" })).toBeInTheDocument();
});

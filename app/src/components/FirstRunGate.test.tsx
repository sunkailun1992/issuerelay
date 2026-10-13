import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { Route, Routes, useNavigate } from "react-router";
import { useSaveSettings, useSettings } from "@/hooks/queries";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { FirstRunGate } from "./FirstRunGate";

const routes = (
  <Routes>
    <Route element={<FirstRunGate />}>
      <Route path="/" element={<p>overview page</p>} />
    </Route>
    <Route path="/onboarding" element={<p>guide page</p>} />
  </Routes>
);

test("a first run goes to the guide", async () => {
  await renderWithProviders(routes, { backend: createMockBackend("fresh", { latencyMs: 0 }) });
  expect(await screen.findByText("guide page")).toBeInTheDocument();
});

test("a configured install shows the page it was asked for", async () => {
  await renderWithProviders(routes);
  expect(await screen.findByText("overview page")).toBeInTheDocument();
});

test("nothing of the page shows while the settings are still being read", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  backend.settings = () => new Promise(() => {});
  await renderWithProviders(routes, { backend });
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(screen.queryByText("overview page")).not.toBeInTheDocument();
  expect(screen.queryByText("guide page")).not.toBeInTheDocument();
});

test("settings that cannot be read show an error with a retry instead of a page", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const read = backend.settings.bind(backend);
  backend.settings = vi
    .fn()
    .mockRejectedValueOnce(new BackendError("the config file is unreadable"))
    .mockImplementation(read);
  await renderWithProviders(routes, { backend });
  expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
  expect(screen.getByText("the config file is unreadable")).toBeInTheDocument();
  expect(screen.queryByText("overview page")).not.toBeInTheDocument();
  expect(screen.queryByText("guide page")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(await screen.findByText("overview page")).toBeInTheDocument();
});

test("finishing the guide gets past the gate it came through, without going back to the guide", async () => {
  // What the guide's last step does: save the settings with the guide completed, then go to the overview.
  function Finish() {
    const settings = useSettings();
    const save = useSaveSettings();
    const navigate = useNavigate();
    const finish = () => {
      if (settings.data) save.mutate({ ...settings.data, onboarding_completed: true }, { onSuccess: () => navigate("/") });
    };
    return (
      <button type="button" onClick={finish}>
        finish the guide
      </button>
    );
  }
  await renderWithProviders(
    <Routes>
      <Route element={<FirstRunGate />}>
        <Route path="/" element={<p>overview page</p>} />
      </Route>
      <Route path="/onboarding" element={<Finish />} />
    </Routes>,
    { backend: createMockBackend("fresh", { latencyMs: 0 }), route: "/onboarding" },
  );
  await userEvent.click(await screen.findByRole("button", { name: "finish the guide" }));
  expect(await screen.findByText("overview page")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "finish the guide" })).not.toBeInTheDocument();
});

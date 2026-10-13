import { onlineManager } from "@tanstack/react-query";
import { act, screen, waitFor } from "@testing-library/react";
import { useStatus } from "@/hooks/queries";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { createQueryClient } from "./query-client";

afterEach(() => {
  onlineManager.setOnline(true);
});

test("every call of the panel is local, so none of them waits for the network or is repeated behind the user's back", () => {
  const { queries, mutations } = createQueryClient().getDefaultOptions();
  expect(queries).toMatchObject({
    // `"online"`, the library's default, pauses every query and action while the system says it is offline.
    networkMode: "always",
    refetchOnReconnect: false,
    refetchOnWindowFocus: false,
    retry: false,
  });
  expect(mutations).toMatchObject({ networkMode: "always", retry: false });
});

test("the test helper renders with the app's own client, so no test runs by other rules than the app", async () => {
  const { client } = await renderWithProviders(<p>anything</p>);
  expect(client.getDefaultOptions()).toEqual(createQueryClient().getDefaultOptions());
});

test("a query reads while the system reports it is offline", async () => {
  onlineManager.setOnline(false);
  function Status() {
    return <p>{useStatus().data?.linear.kind ?? "reading"}</p>;
  }
  await renderWithProviders(<Status />);
  expect(await screen.findByText("connected")).toBeInTheDocument();
});

test("coming back online does not read everything again", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const status = vi.spyOn(backend, "status");
  function Status() {
    return <p>{useStatus().data?.linear.kind ?? "reading"}</p>;
  }
  const { client } = await renderWithProviders(<Status />, { backend });
  await screen.findByText("connected");
  await waitFor(() => expect(client.isFetching()).toBe(0));
  const reads = status.mock.calls.length;
  // The library reads again after it has resumed what was paused, which takes a few ticks.
  await act(async () => onlineManager.setOnline(false));
  await act(async () => onlineManager.setOnline(true));
  expect(status).toHaveBeenCalledTimes(reads);
});

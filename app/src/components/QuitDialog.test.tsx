import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import { renderWithProviders } from "@/test/render";
import { QuitDialog } from "./QuitDialog";

test("quitting with runs in progress asks, and waiting can still become stopping", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let ask: (running: number) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  backend.quit = vi.fn().mockResolvedValue(undefined);
  await renderWithProviders(<QuitDialog />, { backend });
  await act(async () => ask(2));
  expect(screen.getByText(/2 run\(s\) are in progress/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  expect(backend.quit).toHaveBeenCalledWith("wait");
  expect(screen.getByText(/Waiting for the runs to finish/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Stop now" }));
  expect(backend.quit).toHaveBeenLastCalledWith("stop");
});

test("keep running closes the prompt without quitting", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let ask: (running: number) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  backend.quit = vi.fn();
  await renderWithProviders(<QuitDialog />, { backend });
  await act(async () => ask(1));
  await userEvent.click(screen.getByRole("button", { name: "Keep running" }));
  expect(backend.quit).not.toHaveBeenCalled();
  expect(screen.queryByText(/run\(s\) are in progress/)).not.toBeInTheDocument();
});

/** A dialog that the backend asks to open with `running` runs; the answer is whatever `quit` does. */
async function askedWith(running: number, quit: () => Promise<void>) {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let ask: (running: number) => void = () => {};
  backend.onQuitRequested = async (listener) => {
    ask = listener;
    return () => {};
  };
  backend.quit = vi.fn(quit);
  await renderWithProviders(<QuitDialog />, { backend });
  await act(async () => ask(running));
  return { backend, ask: (count: number) => act(async () => ask(count)) };
}

test("escape answers the question like keep running does, without telling the app", async () => {
  const { backend } = await askedWith(1, async () => {});
  await userEvent.keyboard("{Escape}");
  expect(screen.queryByText(/run\(s\) are in progress/)).not.toBeInTheDocument();
  expect(backend.quit).not.toHaveBeenCalled();
});

test("a question asked again after keep running opens the prompt again", async () => {
  const { ask } = await askedWith(1, async () => {});
  await userEvent.click(screen.getByRole("button", { name: "Keep running" }));
  expect(screen.queryByText(/run\(s\) are in progress/)).not.toBeInTheDocument();
  await ask(3);
  expect(screen.getByText(/3 run\(s\) are in progress/)).toBeInTheDocument();
});

test("while waiting, only stopping is offered and escape does not dismiss the prompt", async () => {
  await askedWith(2, async () => {});
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  expect(screen.queryByRole("button", { name: "Keep running" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Wait for completion" })).not.toBeInTheDocument();
  await userEvent.keyboard("{Escape}");
  expect(screen.getByText(/Waiting for the runs to finish/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Stop now" })).toBeInTheDocument();
});

test("a wait the app refuses puts the question back and says why", async () => {
  await askedWith(2, async () => {
    throw new BackendError("the host is shutting down");
  });
  await userEvent.click(screen.getByRole("button", { name: "Wait for completion" }));
  expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
  expect(screen.getByText("the host is shutting down")).toBeInTheDocument();
  expect(screen.getByText(/2 run\(s\) are in progress/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Wait for completion" })).toBeInTheDocument();
});

test("stopping now cannot be asked for twice", async () => {
  const { backend } = await askedWith(2, () => new Promise(() => {}));
  await userEvent.click(screen.getByRole("button", { name: "Stop now" }));
  expect(screen.getByRole("button", { name: "Stop now" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Stop now" }));
  expect(backend.quit).toHaveBeenCalledTimes(1);
});

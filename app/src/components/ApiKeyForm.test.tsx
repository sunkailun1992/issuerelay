import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { queryKeys } from "@/hooks/queries";
import { BackendError } from "@/lib/backend";
import { createMockBackend } from "@/lib/mock";
import type { Viewer } from "@/lib/types";
import { renderWithProviders } from "@/test/render";
import { ApiKeyForm } from "./ApiKeyForm";
import { LinearStatus } from "./LinearStatus";

const FIELD = "Personal API key";

test("the key is typed into a password field that does not offer to remember it", async () => {
  await renderWithProviders(<ApiKeyForm />);
  const input = screen.getByLabelText(FIELD);
  expect(input).toHaveAttribute("type", "password");
  expect(input).toHaveAttribute("autocomplete", "off");
  expect(input).toHaveAttribute("spellcheck", "false");
});

test("the key goes to the engine with the blanks around a paste taken off", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const use = vi.spyOn(backend, "linearUseApiKey");
  await renderWithProviders(<ApiKeyForm />, { backend });
  await userEvent.type(screen.getByLabelText(FIELD), "  lin_api_demo  ");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  await waitFor(() => expect(use).toHaveBeenCalledWith("lin_api_demo"));
  expect(use).toHaveBeenCalledTimes(1);
});

test("Enter in the field uses the key", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const use = vi.spyOn(backend, "linearUseApiKey");
  await renderWithProviders(<ApiKeyForm />, { backend });
  await userEvent.type(screen.getByLabelText(FIELD), "lin_api_demo{Enter}");
  await waitFor(() => expect(use).toHaveBeenCalledWith("lin_api_demo"));
});

test("an empty field cannot be used", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  const use = vi.spyOn(backend, "linearUseApiKey");
  await renderWithProviders(<ApiKeyForm />, { backend });
  const button = screen.getByRole("button", { name: "Use this key" });
  expect(button).toBeDisabled();
  await userEvent.type(screen.getByLabelText(FIELD), "   ");
  expect(button).toBeDisabled();
  await userEvent.type(screen.getByLabelText(FIELD), "{Enter}");
  expect(use).not.toHaveBeenCalled();
});

test("a key that was accepted is cleared from the field, and the page says the key is in use now", async () => {
  await renderWithProviders(<ApiKeyForm />);
  expect(screen.queryByText("Using the API key now")).not.toBeInTheDocument();
  await userEvent.type(screen.getByLabelText(FIELD), "lin_api_demo");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  expect(await screen.findByText("Using the API key now")).toBeInTheDocument();
  expect(screen.getByLabelText(FIELD)).toHaveValue("");
  expect(document.body).not.toHaveTextContent("lin_api_demo");
});

test("a key that was refused is cleared too, is told with the engine's words, and is shown nowhere", async () => {
  await renderWithProviders(<ApiKeyForm />);
  await userEvent.type(screen.getByLabelText(FIELD), "lin_api_bad");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  expect(await screen.findByText("Could not use the API key")).toBeInTheDocument();
  expect(screen.getByText("Linear did not accept the API key")).toBeInTheDocument();
  await waitFor(() => expect(screen.getByLabelText(FIELD)).toHaveValue(""));
  expect(screen.queryByText("Using the API key now")).not.toBeInTheDocument();
  expect(document.body).not.toHaveTextContent("lin_api_bad");
});

test("the key is kept nowhere once it was used, accepted or not", async () => {
  const { client } = await renderWithProviders(<ApiKeyForm />);
  const kept = () => client.getMutationCache().getAll().map((mutation) => mutation.state.variables);
  for (const key of ["lin_api_demo", "lin_api_bad"]) {
    await userEvent.type(screen.getByLabelText(FIELD), key);
    await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
    await waitFor(() => expect(screen.getByLabelText(FIELD)).toHaveValue(""));
    await waitFor(() => expect(kept()).not.toContain(key));
  }
});

test("the line that says the key is in use follows the connection, not the visit", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  await backend.linearUseApiKey("lin_api_demo");
  const { client } = await renderWithProviders(<ApiKeyForm />, { backend });
  expect(await screen.findByText("Using the API key now")).toBeInTheDocument();
  await backend.linearLogin();
  await client.invalidateQueries({ queryKey: queryKeys.status });
  await waitFor(() => expect(screen.queryByText("Using the API key now")).not.toBeInTheDocument());
});

test("a connection made by the browser login does not say it uses the key", async () => {
  const { client } = await renderWithProviders(<ApiKeyForm />);
  await waitFor(() => expect(client.getQueryState(queryKeys.status)?.status).toBe("success"));
  expect(screen.getByRole("button", { name: "Use this key" })).toBeInTheDocument();
  expect(screen.queryByText("Using the API key now")).not.toBeInTheDocument();
});

// core takes one change of the login at a time: while the browser login waits, the key waits too.
test("while a browser login waits the key cannot be used, and the form says why", async () => {
  const backend = createMockBackend("fresh", { latencyMs: 0 });
  let cancel = () => {};
  backend.linearLogin = vi.fn(
    () =>
      new Promise<never>((_, reject) => {
        cancel = () => reject(new BackendError("login_cancelled"));
      }),
  );
  backend.linearCancelLogin = vi.fn(async () => cancel());
  const use = vi.spyOn(backend, "linearUseApiKey");
  await renderWithProviders(
    <>
      <LinearStatus />
      <ApiKeyForm />
    </>,
    { backend },
  );
  await userEvent.type(screen.getByLabelText(FIELD), "lin_api_demo");
  expect(screen.getByRole("button", { name: "Use this key" })).toBeEnabled();
  expect(screen.queryByText("Finish or cancel the browser login first")).not.toBeInTheDocument();

  await userEvent.click(await screen.findByRole("button", { name: "Log in to Linear" }));
  expect(await screen.findByText("Finish or cancel the browser login first")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Use this key" })).toBeDisabled();
  // Enter does not get round it, and what was typed stays for when the login is over.
  await userEvent.type(screen.getByLabelText(FIELD), "{Enter}");
  expect(use).not.toHaveBeenCalled();
  expect(screen.getByLabelText(FIELD)).toHaveValue("lin_api_demo");

  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Use this key" })).toBeEnabled());
  expect(screen.queryByText("Finish or cancel the browser login first")).not.toBeInTheDocument();
});

test("a key being used cannot be used twice", async () => {
  const backend = createMockBackend("demo", { latencyMs: 0 });
  let finish: (() => void) | undefined;
  backend.linearUseApiKey = vi.fn(
    () =>
      new Promise<Viewer>((resolve) => {
        finish = () => resolve({ id: "user-1", name: "Jane Doe" });
      }),
  );
  await renderWithProviders(<ApiKeyForm />, { backend });
  await userEvent.type(screen.getByLabelText(FIELD), "lin_api_demo");
  await userEvent.click(screen.getByRole("button", { name: "Use this key" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Use this key" })).toBeDisabled());
  await userEvent.type(screen.getByLabelText(FIELD), "{Enter}");
  expect(backend.linearUseApiKey).toHaveBeenCalledTimes(1);
  finish?.();
  await waitFor(() => expect(screen.getByLabelText(FIELD)).toHaveValue(""));
});

import { Inbox } from "lucide-react";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { BackendError } from "@/lib/backend";
import { renderWithProviders } from "@/test/render";
import { DetailLine, EmptyState, ErrorState, LoadingState } from "./states";

test("errors show a translated headline, the backend's line as a cleaned detail, and a retry", async () => {
  const retry = vi.fn();
  await renderWithProviders(
    <ErrorState error={new BackendError("Linear did not answer `list_issues` within 30s\u001b[0m")} onRetry={retry} />,
  );
  expect(screen.getByText("Something went wrong")).toBeInTheDocument();
  expect(screen.getByText("Linear did not answer `list_issues` within 30s")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(retry).toHaveBeenCalled();
});

test("an error the backend answers with as a code is translated and shows no raw text", async () => {
  await renderWithProviders(<ErrorState error={new BackendError("not_a_linear_link")} />);
  expect(screen.getByText("IssueRelay only opens Linear's own pages")).toBeInTheDocument();
  expect(screen.queryByText("not_a_linear_link")).not.toBeInTheDocument();
});

test("an error state takes its headline as a translation key, and has no retry unless it is given one", async () => {
  await renderWithProviders(<ErrorState error={new Error("disk is full")} headline="actions.failed.save_settings" />);
  expect(screen.getByRole("alert")).toHaveTextContent("Could not save the settings");
  expect(screen.getByText("disk is full")).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

test("an error without a message has a headline only", async () => {
  await renderWithProviders(<ErrorState error={new BackendError("")} />);
  expect(screen.getByRole("alert")).toHaveTextContent(/^Something went wrong$/);
});

test("an empty state shows its title, description and action", async () => {
  await renderWithProviders(
    <EmptyState
      icon={Inbox}
      title="Nothing here yet"
      description="Runs appear when an issue is picked up."
      action={<button type="button">Open Linear</button>}
    />,
  );
  expect(screen.getByText("Nothing here yet")).toBeInTheDocument();
  expect(screen.getByText("Runs appear when an issue is picked up.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Open Linear" })).toBeInTheDocument();
});

test("an empty state without an action has no button", async () => {
  await renderWithProviders(<EmptyState icon={Inbox} title="Nothing here yet" description="Come back later." />);
  expect(screen.getByText("Come back later.")).toBeInTheDocument();
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

test("a loading state announces itself and draws three rows", async () => {
  const { container } = await renderWithProviders(<LoadingState />);
  expect(screen.getByRole("status", { name: "Loading…" })).toBeInTheDocument();
  expect(container.querySelectorAll("[data-slot=skeleton]")).toHaveLength(3);
});

test("a loading state draws as many rows as it is asked for", async () => {
  const { container } = await renderWithProviders(<LoadingState rows={5} />);
  expect(container.querySelectorAll("[data-slot=skeleton]")).toHaveLength(5);
});

test("a detail line shows the engine's text cleaned of escape sequences and line breaks", async () => {
  await renderWithProviders(<DetailLine text={"exit code 1\u001b[2J\nsecond\tline"} />);
  expect(screen.getByText("exit code 1 second line")).toBeInTheDocument();
});

test("a detail line with nothing left after cleaning shows nothing", async () => {
  const { container } = await renderWithProviders(<DetailLine text={"\u001b[0m   "} />);
  expect(container.querySelector("p")).toBeNull();
});

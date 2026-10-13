import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useRef, useState } from "react";
import { vi } from "vitest";
import { renderWithProviders } from "@/test/render";
import { FullAccessWarning } from "./FullAccessWarning";

test("it says what full access allows and nothing else happens until the user answers", async () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  await renderWithProviders(<FullAccessWarning open onConfirm={onConfirm} onCancel={onCancel} />);
  const dialog = await screen.findByRole("alertdialog", { name: "Full access runs without limits" });
  expect(dialog).toHaveTextContent(/run any command on this computer and use all of your credentials/);
  // The way out is part of the warning: a lower level.
  expect(dialog).toHaveTextContent(/choose a lower permission level/);
  expect(onConfirm).not.toHaveBeenCalled();
  expect(onCancel).not.toHaveBeenCalled();
});

test("nothing is shown while it is closed", async () => {
  await renderWithProviders(<FullAccessWarning open={false} onConfirm={() => {}} onCancel={() => {}} />);
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});

test("understanding it confirms, and does not also count as cancelling", async () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  await renderWithProviders(<FullAccessWarning open onConfirm={onConfirm} onCancel={onCancel} />);
  await userEvent.click(await screen.findByRole("button", { name: "I understand" }));
  expect(onConfirm).toHaveBeenCalledTimes(1);
  expect(onCancel).not.toHaveBeenCalled();
});

test("cancelling and the Escape key both say no", async () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  await renderWithProviders(<FullAccessWarning open onConfirm={onConfirm} onCancel={onCancel} />);
  await userEvent.click(await screen.findByRole("button", { name: "Cancel" }));
  expect(onCancel).toHaveBeenCalledTimes(1);
  await userEvent.keyboard("{Escape}");
  expect(onCancel).toHaveBeenCalledTimes(2);
  expect(onConfirm).not.toHaveBeenCalled();
});

test("the focus starts on the safe answer, so that Enter does not confirm by accident", async () => {
  await renderWithProviders(<FullAccessWarning open onConfirm={() => {}} onCancel={() => {}} />);
  const cancel = await screen.findByRole("button", { name: "Cancel" });
  expect(cancel).toHaveFocus();
});

/**
 * A page with a button that asks the warning, and a second button that the warning may be told to
 * go back to. `removeAsker` takes the first button away while the warning is open.
 */
function Page({ withPlace = false, removeAsker = false }: { withPlace?: boolean; removeAsker?: boolean }) {
  const [open, setOpen] = useState(false);
  const place = useRef<HTMLButtonElement>(null);
  return (
    <>
      {!(open && removeAsker) && (
        <button type="button" onClick={() => setOpen(true)}>
          Ask
        </button>
      )}
      <button type="button" ref={place}>
        Elsewhere
      </button>
      <FullAccessWarning open={open} onConfirm={() => setOpen(false)} onCancel={() => setOpen(false)} returnFocusTo={withPlace ? place : undefined} />
    </>
  );
}

test.each(["Cancel", "Escape", "I understand"])("closing the warning with %s returns the focus to what had it when the warning opened", async (how) => {
  await renderWithProviders(<Page />);
  const ask = screen.getByRole("button", { name: "Ask" });
  await userEvent.click(ask);
  await screen.findByRole("alertdialog");
  expect(ask).not.toHaveFocus();
  if (how === "Escape") await userEvent.keyboard("{Escape}");
  else await userEvent.click(screen.getByRole("button", { name: how }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  await waitFor(() => expect(ask).toHaveFocus());
});

test("the place the caller names is only used when what had the focus is gone", async () => {
  // It comes second: while what had the focus is there, that is where the focus returns.
  const first = await renderWithProviders(<Page withPlace />);
  const ask = screen.getByRole("button", { name: "Ask" });
  await userEvent.click(ask);
  await screen.findByRole("alertdialog");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(ask).toHaveFocus());
  expect(screen.getByRole("button", { name: "Elsewhere" })).not.toHaveFocus();
  first.unmount();
  // The button that asked went away while the warning was open: the named place gets the focus.
  await renderWithProviders(<Page withPlace removeAsker />);
  await userEvent.click(screen.getByRole("button", { name: "Ask" }));
  await screen.findByRole("alertdialog");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.getByRole("button", { name: "Elsewhere" })).toHaveFocus());
});

test("with nothing to return to the warning closes and the focus is left alone", async () => {
  await renderWithProviders(<Page removeAsker />);
  await userEvent.click(screen.getByRole("button", { name: "Ask" }));
  await screen.findByRole("alertdialog");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  expect(document.activeElement).toBe(document.body);
});

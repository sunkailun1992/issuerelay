import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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

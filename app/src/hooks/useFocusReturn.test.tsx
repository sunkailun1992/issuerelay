import { renderHook } from "@testing-library/react";
import { useFocusReturn } from "./useFocusReturn";

/** A button on the page, with the focus on it when `focused`. */
function button(label: string, focused = false): HTMLButtonElement {
  const element = document.createElement("button");
  element.textContent = label;
  document.body.appendChild(element);
  if (focused) element.focus();
  return element;
}

afterEach(() => {
  document.body.replaceChildren();
});

const closing = () => new Event("focusScope.autoFocusOnUnmount", { cancelable: true });

/** Opens a dialog (the hook's `open` turns true) with the focus where it is, moves the focus into the dialog, and closes it. */
function openAndClose(fallback?: () => HTMLElement | null) {
  const hook = renderHook(({ open }) => useFocusReturn(open, fallback), { initialProps: { open: false } });
  hook.rerender({ open: true });
  const inside = button("inside the dialog", true);
  const event = closing();
  hook.result.current(event);
  inside.remove();
  return event;
}

test("the focus goes back to the element that had it when the dialog opened", () => {
  const opener = button("opener", true);
  const event = openAndClose();
  expect(opener).toHaveFocus();
  // The library's own return of focus (to a trigger there is none of) is what is switched off.
  expect(event.defaultPrevented).toBe(true);
});

test("the opener comes before the fallback", () => {
  const opener = button("opener", true);
  const fallback = button("fallback");
  openAndClose(() => fallback);
  expect(opener).toHaveFocus();
});

test("when the opener is gone the fallback gets the focus", () => {
  const opener = button("opener", true);
  const fallback = button("fallback");
  const hook = renderHook(({ open }) => useFocusReturn(open, () => fallback), { initialProps: { open: false } });
  hook.rerender({ open: true });
  button("inside the dialog", true);
  // What opened the dialog went away while it was open.
  opener.remove();
  hook.result.current(closing());
  expect(fallback).toHaveFocus();
});

test("with no element focused when the dialog opened, there is no opener: the fallback is used", () => {
  expect(document.activeElement).toBe(document.body);
  const fallback = button("fallback");
  openAndClose(() => fallback);
  expect(fallback).toHaveFocus();
});

test("a fallback that names nothing leaves the focus alone, and so does having none", () => {
  openAndClose(() => null);
  expect(document.activeElement).toBe(document.body);
  const event = openAndClose();
  expect(event.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(document.body);
});

test("the opener is the element that had the focus at the moment the dialog opened, not at some later render", () => {
  const first = button("first", true);
  const second = button("second");
  const hook = renderHook(({ open }) => useFocusReturn(open), { initialProps: { open: false } });
  hook.rerender({ open: true });
  button("inside the dialog", true);
  // A later render with the dialog still open does not take the focus inside as the opener.
  hook.rerender({ open: true });
  second.focus();
  hook.result.current(closing());
  expect(first).toHaveFocus();
});

test("each opening remembers its own opener", () => {
  const one = button("one", true);
  const two = button("two");
  const hook = renderHook(({ open }) => useFocusReturn(open), { initialProps: { open: false } });
  hook.rerender({ open: true });
  button("inside", true);
  hook.result.current(closing());
  hook.rerender({ open: false });
  expect(one).toHaveFocus();
  two.focus();
  hook.rerender({ open: true });
  button("inside again", true);
  hook.result.current(closing());
  expect(two).toHaveFocus();
});

/**
 * A button that opens a list (or menu), the list with its items, and the first item with the focus.
 * While the list is open the button names it in `aria-controls`.
 */
function list(role: "listbox" | "menu", item: "option" | "menuitem", { named = true } = {}) {
  const trigger = button("opens the list", true);
  const popup = document.createElement("div");
  popup.setAttribute("role", role);
  popup.id = `list-${role}`;
  if (named) trigger.setAttribute("aria-controls", popup.id);
  const choice = document.createElement("div");
  choice.setAttribute("role", item);
  choice.tabIndex = -1;
  popup.appendChild(choice);
  document.body.appendChild(popup);
  choice.focus();
  return { trigger, popup, choice };
}

test.each([
  ["listbox", "option"],
  ["menu", "menuitem"],
] as const)("an item chosen from a %s that is closing leaves the list's button as the opener", (role, item) => {
  // Choosing from a list closes it, and the dialog opens in the same moment: the item has the
  // focus when the dialog opens, and is gone when it closes. The button is where the user was.
  const hook = renderHook(({ open }) => useFocusReturn(open), { initialProps: { open: false } });
  const { trigger, choice } = list(role, item);
  expect(choice).toHaveFocus();
  // The list is closing: the button no longer names it.
  trigger.removeAttribute("aria-controls");
  hook.rerender({ open: true });
  button("inside the dialog", true);
  choice.remove();
  hook.result.current(closing());
  expect(trigger).toHaveFocus();
});

test("an item of a list whose button never named it is itself the opener, and the fallback takes over when it is gone", () => {
  const fallback = button("fallback");
  const hook = renderHook(({ open }) => useFocusReturn(open, () => fallback), { initialProps: { open: false } });
  const { choice } = list("listbox", "option", { named: false });
  hook.rerender({ open: true });
  button("inside the dialog", true);
  choice.remove();
  hook.result.current(closing());
  expect(fallback).toHaveFocus();
});

test("a button that names several lists is the button of each of them", () => {
  const hook = renderHook(({ open }) => useFocusReturn(open), { initialProps: { open: false } });
  const popup = document.createElement("div");
  popup.setAttribute("role", "listbox");
  popup.id = "second-list";
  const choice = document.createElement("div");
  choice.tabIndex = -1;
  popup.appendChild(choice);
  document.body.appendChild(popup);
  const trigger = button("opens two lists");
  trigger.setAttribute("aria-controls", "first-list second-list");
  choice.focus();
  trigger.removeAttribute("aria-controls");
  hook.rerender({ open: true });
  button("inside the dialog", true);
  choice.remove();
  hook.result.current(closing());
  expect(trigger).toHaveFocus();
});

test("a list that had the focus long ago does not take the place of an opener that is a plain button", () => {
  const hook = renderHook(({ open }) => useFocusReturn(open), { initialProps: { open: false } });
  const { trigger, choice } = list("listbox", "option");
  trigger.removeAttribute("aria-controls");
  choice.remove();
  // Later the user is on another button, and it opens the dialog.
  const other = button("another button", true);
  hook.rerender({ open: true });
  button("inside the dialog", true);
  hook.result.current(closing());
  expect(other).toHaveFocus();
  expect(trigger).not.toHaveFocus();
});

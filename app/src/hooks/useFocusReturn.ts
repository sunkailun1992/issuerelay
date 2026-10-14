import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** The popups a choice can be made from, which close once it is made. */
const POPUP = '[role="listbox"], [role="menu"]';

/** The button that opened `popup`: the one whose `aria-controls` names it. Only an open popup is named. */
function buttonOf(popup: Element): HTMLElement | null {
  if (popup.id === "") return null;
  for (const element of document.querySelectorAll<HTMLElement>("[aria-controls]")) {
    if (element.getAttribute("aria-controls")?.split(/\s+/).includes(popup.id)) return element;
  }
  return null;
}

/**
 * Gives the focus back to where it was when a dialog that is open because its page says so
 * (`open`, with no trigger of its own) closes. Radix returns the focus to the dialog's trigger, a
 * dialog without one has none to return it to, and the focus fell to the document: a keyboard or
 * screen reader user began again from the top of the page. So the element that has the focus
 * when the dialog opens is remembered (in a layout effect, which runs before Radix moves the focus
 * into the dialog) and gets it back; when it is gone, `fallback` names another place. Pass the
 * returned function as the dialog content's `onCloseAutoFocus`.
 *
 * A dialog can open because something was chosen from a list or a menu. The focus is on the item
 * chosen then, the list is closing and takes the item with it, and what is wanted is the list's
 * button. By the time the dialog opens the button no longer names the list (it does only while
 * the list is open), so which button opened a list is noted whenever the focus is in it.
 */
export function useFocusReturn(open: boolean, fallback?: () => HTMLElement | null): (event: Event) => void {
  const opener = useRef<HTMLElement | null>(null);
  const [buttons] = useState(() => new WeakMap<Element, HTMLElement>());
  useEffect(() => {
    const onFocusIn = (event: FocusEvent) => {
      if (!(event.target instanceof Element)) return;
      const popup = event.target.closest(POPUP);
      const button = popup === null ? null : buttonOf(popup);
      if (popup !== null && button !== null) buttons.set(popup, button);
    };
    document.addEventListener("focusin", onFocusIn, true);
    return () => document.removeEventListener("focusin", onFocusIn, true);
  }, [buttons]);
  useLayoutEffect(() => {
    if (!open) return;
    const active = document.activeElement;
    if (!(active instanceof HTMLElement) || active === document.body) {
      opener.current = null;
      return;
    }
    const popup = active.closest(POPUP);
    opener.current = (popup === null ? undefined : buttons.get(popup)) ?? active;
  }, [open, buttons]);
  return useCallback(
    (event: Event) => {
      // The library would return the focus to the trigger it does not have.
      event.preventDefault();
      const remembered = opener.current;
      const target = remembered?.isConnected ? remembered : (fallback?.() ?? null);
      target?.focus();
    },
    [fallback],
  );
}

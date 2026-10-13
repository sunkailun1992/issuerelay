import { act } from "@testing-library/react";
import { vi } from "vitest";

/**
 * jsdom has no animations, so a Radix dialog that closes is gone at once. In the panel it stays
 * for the length of its fade, 100 ms, and its buttons can still be clicked meanwhile. This makes
 * jsdom say what a browser says: an element whose `data-state` is "open" has an animation named
 * "enter", one that is "closed" has one named "exit" (Radix waits for the end of an animation that
 * has changed), and `endFades` ends them. The stand-in is undone by `restoreMocks` after the test.
 */
export function holdClosingDialogs(): void {
  const real = window.getComputedStyle.bind(window);
  vi.spyOn(window, "getComputedStyle").mockImplementation((element, pseudo) => {
    const style = real(element, pseudo);
    if (element.getAttribute("data-state") === null) return style;
    return new Proxy(style, {
      get(target, property) {
        // Read now, not when the style was asked for: Radix keeps the object it got at the start.
        if (property === "animationName") return element.getAttribute("data-state") === "closed" ? "exit" : "enter";
        const value: unknown = Reflect.get(target, property);
        return typeof value === "function" ? value.bind(target) : value;
      },
    });
  });
}

/** Ends the fade of everything that is closing. */
export async function endFades(): Promise<void> {
  await act(async () => {
    for (const element of document.querySelectorAll('[data-state="closed"]')) {
      const end = new Event("animationend", { bubbles: true });
      Object.defineProperty(end, "animationName", { value: "exit" });
      element.dispatchEvent(end);
    }
  });
}

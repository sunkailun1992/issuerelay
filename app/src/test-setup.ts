import "@testing-library/jest-dom/vitest";
import { setFixedClock } from "@/lib/clock";

// Radix (Select, Switch, ScrollArea, Dialog, RadioGroup) measures elements, captures the pointer
// and scrolls options into view; jsdom implements none of these.
if (!("ResizeObserver" in globalThis)) {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
}
Element.prototype.hasPointerCapture = () => false;
Element.prototype.setPointerCapture = () => {};
Element.prototype.releasePointerCapture = () => {};
Element.prototype.scrollIntoView = () => {};
// The theme follows `prefers-color-scheme`; jsdom has no matchMedia.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList,
});

afterEach(() => {
  // A test that fixed the clock (the mock backend does) never leaks it into the next one, even
  // when its assertions failed.
  setFixedClock(null);
});

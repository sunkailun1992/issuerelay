import "@testing-library/jest-dom/vitest";
import { toast } from "sonner";
import { setFixedClock } from "@/lib/clock";
import { initI18n } from "@/i18n";

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

// A React update outside act() means a test looked at a screen that was still changing, or left
// work running when it ended: React says so on console.error, and the test fails for it (below).
const actWarnings: string[] = [];
const printError = console.error.bind(console);
console.error = (...args: unknown[]) => {
  const message = String(args[0]);
  if (message.includes("not wrapped in act(")) actWarnings.push(message.split("\n")[0].replace("%s", String(args[1])));
  printError(...args);
};

afterEach(() => {
  // A test that fixed the clock (the mock backend does) never leaks it into the next one, even
  // when its assertions failed.
  setFixedClock(null);
  // sonner keeps its toasts in a store of its own, outside React, and replays the ones still
  // active to the next Toaster that mounts: without this a failure toast of one test shows up in
  // the next test's page.
  toast.dismiss();
  // This hook was registered first, so it runs after the test file's own and after the cleanup
  // (Vitest runs "after" hooks in reverse): updates made while those run are counted too.
  const late = actWarnings.splice(0);
  if (late.length > 0) throw new Error(`a React update was not wrapped in act(): ${late.join("; ")}`);
});

// Every test runs with i18next started once: a `t()` key that English lacks fails the test
// instead of showing the key, and so does one that names a group of texts (`t("nav")`), for which
// i18next would show a sentence about the key. (Missing in another language only is caught by
// i18n.test.ts.)
await initI18n("en", {
  saveMissing: true,
  missingKeyHandler: (_languages, _namespace, key) => {
    throw new Error(`missing translation key: ${key}`);
  },
  returnedObjectHandler: (key) => {
    throw new Error(`translation key names a group of texts, not a text: ${key}`);
  },
});

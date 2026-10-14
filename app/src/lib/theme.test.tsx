import { act, render } from "@testing-library/react";
import { vi } from "vitest";
import { useSystemTheme } from "./theme";

function Probe() {
  useSystemTheme();
  return null;
}

/** A `prefers-color-scheme: dark` query the test can flip, and the listeners left on it. */
function systemAppearance(dark: boolean) {
  let matches = dark;
  const listeners = new Set<() => void>();
  vi.spyOn(window, "matchMedia").mockImplementation(
    (query) =>
      ({
        get matches() {
          return matches;
        },
        media: query,
        addEventListener: (_type: string, listener: () => void) => listeners.add(listener),
        removeEventListener: (_type: string, listener: () => void) => listeners.delete(listener),
      }) as unknown as MediaQueryList,
  );
  return {
    listeners,
    set(next: boolean) {
      matches = next;
      listeners.forEach((listener) => listener());
    },
  };
}

afterEach(() => {
  document.documentElement.classList.remove("dark");
});

test("a dark system gets the dark class and a light one does not", () => {
  systemAppearance(true);
  const dark = render(<Probe />);
  expect(document.documentElement).toHaveClass("dark");
  dark.unmount();
  document.documentElement.classList.remove("dark");
  systemAppearance(false);
  render(<Probe />);
  expect(document.documentElement).not.toHaveClass("dark");
});

test("the class follows the system while the panel is open", () => {
  const system = systemAppearance(false);
  render(<Probe />);
  act(() => system.set(true));
  expect(document.documentElement).toHaveClass("dark");
  act(() => system.set(false));
  expect(document.documentElement).not.toHaveClass("dark");
});

test("closing the panel stops listening", () => {
  const system = systemAppearance(false);
  const { unmount } = render(<Probe />);
  expect(system.listeners.size).toBe(1);
  unmount();
  expect(system.listeners.size).toBe(0);
});

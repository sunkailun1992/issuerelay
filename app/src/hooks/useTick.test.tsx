import { act, render } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { useTick } from "./useTick";

/** Calls `onRender` every time it is rendered. */
function Counter({ periodMs, onRender }: { periodMs: number; onRender: () => void }) {
  useTick(periodMs);
  onRender();
  return null;
}

afterEach(() => {
  vi.useRealTimers();
});

test("the component renders again on every tick, and the timer ends with it", () => {
  vi.useFakeTimers();
  const onRender = vi.fn();
  const { unmount } = render(<Counter periodMs={1000} onRender={onRender} />);
  expect(onRender).toHaveBeenCalledTimes(1);
  // One tick at a time: ticks that fire inside one act() are rendered together.
  for (let second = 0; second < 3; second += 1) {
    act(() => {
      vi.advanceTimersByTime(1000);
    });
  }
  expect(onRender).toHaveBeenCalledTimes(4);
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
});

test("a new period replaces the old timer instead of adding one", () => {
  vi.useFakeTimers();
  const onRender = vi.fn();
  const { rerender } = render(<Counter periodMs={1000} onRender={onRender} />);
  rerender(<Counter periodMs={5000} onRender={onRender} />);
  expect(vi.getTimerCount()).toBe(1);
  act(() => {
    vi.advanceTimersByTime(4000);
  });
  // Only the two renders so far: the old one-second timer is gone and the new one has not fired.
  expect(onRender).toHaveBeenCalledTimes(2);
  act(() => {
    vi.advanceTimersByTime(1000);
  });
  expect(onRender).toHaveBeenCalledTimes(3);
});

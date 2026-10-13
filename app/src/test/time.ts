import { act } from "@testing-library/react";
import { vi } from "vitest";

/**
 * Moves fake time forward in `step` milliseconds until `done()` holds and returns how far it had
 * to go, so that a test says what must have happened by when without waiting in real time and
 * without knowing the library's exact timer arithmetic (a timer set while time advances comes due
 * a millisecond later). Fails when `limit` milliseconds of fake time pass without it holding.
 * Fake timers must be on, and they must cover `setTimeout` too (the mock backend answers from one).
 */
export async function advanceUntil(done: () => boolean, { step = 100, limit = 10_000 } = {}): Promise<number> {
  let spent = 0;
  while (!done()) {
    if (spent >= limit) throw new Error(`not done after ${limit} ms of fake time`);
    await act(() => vi.advanceTimersByTimeAsync(step));
    spent += step;
  }
  return spent;
}

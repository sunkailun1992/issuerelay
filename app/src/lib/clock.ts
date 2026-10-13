/** The mock backend's fixed "now", so relative times in tests and screenshots do not drift. */
export const MOCK_NOW = new Date("2026-10-12T09:00:00Z");

let fixed: Date | null = null;

/** Fixes `now()` at `at`; `null` goes back to the real clock. The test setup resets it after each test. */
export function setFixedClock(at: Date | null): void {
  fixed = at;
}

export function now(): Date {
  return fixed ?? new Date();
}

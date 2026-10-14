import { MOCK_NOW, now, setFixedClock } from "@/lib/clock";

test("now() reads the real clock until it is fixed", () => {
  const before = Date.now();
  const read = now().getTime();
  expect(read).toBeGreaterThanOrEqual(before);
  expect(read).toBeLessThanOrEqual(Date.now());
});

test("setFixedClock fixes now() and null goes back to the real clock", () => {
  setFixedClock(MOCK_NOW);
  expect(now()).toBe(MOCK_NOW);
  setFixedClock(null);
  expect(now().getTime()).not.toBe(MOCK_NOW.getTime());
});

// The next two tests rely on running in this order: the first leaves the clock fixed on purpose,
// the second shows that the shared setup in test-setup.ts reset it.
test("a test may leave the clock fixed", () => {
  setFixedClock(MOCK_NOW);
  expect(now()).toBe(MOCK_NOW);
});

test("the shared setup resets the fixed clock after every test", () => {
  expect(now().getTime()).not.toBe(MOCK_NOW.getTime());
});

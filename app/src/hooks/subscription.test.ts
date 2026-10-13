import { vi } from "vitest";
import { subscription } from "./subscription";

test("cleaning up after the subscription resolved unsubscribes once", async () => {
  const stop = vi.fn();
  const cleanup = subscription(Promise.resolve(stop));
  await Promise.resolve();
  expect(stop).not.toHaveBeenCalled();
  cleanup();
  expect(stop).toHaveBeenCalledTimes(1);
});

test("cleaning up before the subscription resolved unsubscribes as soon as it does", async () => {
  let resolve: (stop: () => void) => void = () => {};
  const start = new Promise<() => void>((done) => {
    resolve = done;
  });
  const stop = vi.fn();
  const cleanup = subscription(start);
  cleanup();
  expect(stop).not.toHaveBeenCalled();
  resolve(stop);
  await start;
  expect(stop).toHaveBeenCalledTimes(1);
});

test("a subscription that fails is logged, not left as an unhandled rejection", async () => {
  const log = vi.spyOn(console, "error").mockImplementation(() => {});
  const cleanup = subscription(Promise.reject(new Error("listen is not allowed")));
  await vi.waitFor(() => expect(log).toHaveBeenCalled());
  expect(String(log.mock.calls[0].join(" "))).toContain("listen is not allowed");
  expect(() => cleanup()).not.toThrow();
});

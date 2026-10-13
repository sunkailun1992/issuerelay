import { expect, test } from "vitest";
import cases from "./mask-cases.json";
import { maskCredentials } from "./mask";

test.each(cases)("$value is shown as $masked, as the engine masks it", ({ value, masked }) => {
  expect(maskCredentials(value)).toBe(masked);
});

// Core's own tests hold the same two properties of its function; the panel masks what the engine
// has masked already, so a second masking must leave it alone.
test("masking a masked value changes nothing", () => {
  for (const { masked } of cases) {
    expect(maskCredentials(masked)).toBe(masked);
  }
});

test("no case shows the password it hides", () => {
  for (const { value } of cases) {
    expect(maskCredentials(value)).not.toContain("s3cret");
  }
});

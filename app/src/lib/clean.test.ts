import { expect, test } from "vitest";
import cases from "./clean-cases.json";
import { cleanDisplayLines, cleanDisplayText } from "./clean";

// Each row names the code points it uses in `note`; the file writes them as escapes.
test.each(cases)("$note", ({ input, expected }) => {
  expect(cleanDisplayText(input)).toBe(expected);
});

test("multi-line output keeps its lines, each one cleaned", () => {
  // U+001B [0m resets a colour; U+202E (right-to-left override) is removed.
  expect(cleanDisplayLines("ok\u001b[0m\r\n\u202efailed\tonce")).toBe("ok\nfailed once");
});

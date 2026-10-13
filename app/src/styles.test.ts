import { expect, test } from "vitest";
import css from "./styles.css?raw";
import { JOB_STATES } from "@/lib/types";
import { STATE_STYLE } from "@/components/StatusBadge";

/** The declarations of the first rule with this selector. */
function block(selector: string): string {
  const start = css.indexOf(`${selector} {`);
  expect(start, `${selector} block`).toBeGreaterThanOrEqual(0);
  return css.slice(start, css.indexOf("\n}", start));
}

// The state badges name colours by class (text-state-running ...); a class without a variable
// would simply not be styled, and nothing else would notice.
test("every state colour the badges use is defined for light and dark and offered to Tailwind", () => {
  const names = JOB_STATES.map((state) => STATE_STYLE[state].className.split(" ")[0].replace("text-state-", ""));
  expect(new Set(names).size).toBe(JOB_STATES.length);
  const theme = block("@theme inline");
  for (const name of names) {
    expect(block(":root"), `light --state-${name}`).toMatch(new RegExp(`--state-${name}: oklch\\(`));
    expect(block(".dark"), `dark --state-${name}`).toMatch(new RegExp(`--state-${name}: oklch\\(`));
    expect(theme, `theme --color-state-${name}`).toContain(`--color-state-${name}: var(--state-${name});`);
  }
});

test("the layout constants exist once, as variables, for pages to refer to", () => {
  const root = block(":root");
  for (const constant of ["--sidebar-width", "--content-max-width", "--page-padding"]) {
    expect(root, constant).toContain(`${constant}:`);
  }
});

test("the interface uses the system fonts, which cover Chinese, Japanese and Korean", () => {
  const theme = block("@theme inline");
  expect(theme).toMatch(/--font-sans: -apple-system, .*"PingFang SC".*"Hiragino Sans".*"Microsoft YaHei UI".*"Malgun Gothic"/);
  expect(theme).toMatch(/--font-mono: .*"SF Mono".*Consolas/);
  expect(css).not.toContain("fontsource");
});

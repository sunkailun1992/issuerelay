import { describe, expect, test } from "vitest";
import css from "./styles.css?raw";
import buttonSource from "@/components/ui/button.tsx?raw";
import { JOB_STATES } from "@/lib/types";
import { STATE_STYLE } from "@/components/StatusBadge";
import { blend, bytes, contrast, inGamut, oklch } from "@/test/colour";

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

// On Windows (WebView2) the Apple and Segoe UI families have no CJK glyphs and PingFang and
// Hiragino are missing: the first CJK font of the stack draws Japanese and Traditional Chinese too.
test("Japanese, Traditional Chinese and Korean get Windows fonts of their own", () => {
  const families = (selector: string) => {
    const match = /--font-sans: ([^;]*);/.exec(block(selector));
    expect(match, `${selector} --font-sans`).not.toBeNull();
    return match![1].split(",").map((family) => family.trim().replaceAll('"', ""));
  };
  const cjkOf = (list: string[]) => list.filter((family) => /YaHei|JhengHei|Yu Gothic|Meiryo|Malgun|PingFang|Hiragino|Apple SD/.test(family));
  expect(cjkOf(families(":root:lang(ja)"))).toEqual(["Hiragino Sans", "Yu Gothic UI", "Meiryo UI"]);
  expect(cjkOf(families(":root:lang(zh-TW)"))).toEqual(["PingFang TC", "Microsoft JhengHei UI"]);
  expect(cjkOf(families(":root:lang(ko)"))).toEqual(["Apple SD Gothic Neo", "Malgun Gothic"]);
  // The page's font is the variable, which those rules set for the language of <html>.
  expect(block("@layer base")).toMatch(/html \{[^}]*font-family: var\(--font-sans\);/);
});

/** The colour a theme gives to `--name`, as `oklch(...)` text. */
function token(theme: ":root" | ".dark", name: string): string {
  const match = new RegExp(`^\\s*--${name}: (oklch\\([^)]*\\));`, "m").exec(block(theme));
  expect(match, `${theme} --${name}`).not.toBeNull();
  return match![1];
}

const THEMES = [
  ["light", ":root"],
  ["dark", ".dark"],
] as const;
const STATES = JOB_STATES.map((state) => STATE_STYLE[state].className.split(" ")[0].replace("text-state-", ""));
/** What a badge can sit on: the page, a card, and the muted fill of a hovered row or a block. */
const SURFACES = ["background", "card", "muted"] as const;
/** A badge's tint is its own colour at 10% over the surface (`bg-state-*` with `/10`). */
const TINT = 0.1;

describe("the state colours", () => {
  // Pixels Chrome 154 drew on 2026-10-13 for the same colours (screenshot of a page of swatches).
  // They tie the maths below to what a user sees, and settle how the 10% tint is composited.
  test("the colour maths gives the pixels a browser draws", () => {
    const near = (got: number[], want: number[]) => got.every((channel, i) => Math.abs(channel - want[i]) <= 1);
    expect(bytes(oklch("oklch(0.72 0.15 250)"))).toEqual([82, 169, 254]);
    expect(bytes(oklch("oklch(0.60 0.15 150)"))).toEqual([37, 152, 77]);
    const onDark = oklch("oklch(0.205 0 0)");
    const white = oklch("oklch(1 0 0)");
    expect(near(bytes(blend(oklch("oklch(0.72 0.15 250)"), onDark, TINT)), [28, 37, 46])).toBe(true);
    expect(near(bytes(blend(oklch("oklch(0.75 0.14 150)"), onDark, TINT)), [30, 40, 33])).toBe(true);
    expect(near(bytes(blend(oklch("oklch(0.60 0.15 150)"), white, TINT)), [233, 244, 237])).toBe(true);
    expect(near(bytes(blend(oklch("oklch(0.55 0.20 295)"), white, TINT)), [242, 237, 251])).toBe(true);
  });

  test("the contrast of black on white is 21 and of a colour on itself 1", () => {
    expect(contrast(oklch("oklch(0 0 0)"), oklch("oklch(1 0 0)"))).toBeCloseTo(21, 5);
    expect(contrast(oklch("oklch(0.6 0.1 30)"), oklch("oklch(0.6 0.1 30)"))).toBe(1);
  });

  test.each(THEMES)("%s: every state colour is inside the sRGB gamut, so every display shows it as measured", (_theme, selector) => {
    for (const name of STATES) {
      expect(inGamut(oklch(token(selector, `state-${name}`))), `${selector} --state-${name}`).toBe(true);
    }
  });

  // Small text on a badge: WCAG 1.4.3 asks for 4.5:1.
  test.each(THEMES)("%s: the label of every badge reaches 4.5:1 on its own tint over each surface", (_theme, selector) => {
    for (const name of STATES) {
      const text = oklch(token(selector, `state-${name}`));
      for (const surface of SURFACES) {
        const under = oklch(token(selector, surface));
        const ratio = contrast(text, blend(text, under, TINT));
        expect(ratio, `${selector} --state-${name} on ${surface}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  // Blending light itself, which reviews of the colours did, reads lower over a dark surface; the
  // colours clear that too on the two surfaces the pages use most.
  test.each(THEMES)("%s: the label also reaches 4.5:1 when the tint is blended as light", (_theme, selector) => {
    for (const name of STATES) {
      const text = oklch(token(selector, `state-${name}`));
      for (const surface of ["background", "card"] as const) {
        const under = oklch(token(selector, surface));
        const ratio = contrast(text, blend(text, under, TINT, "linear"));
        expect(ratio, `${selector} --state-${name} on ${surface}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
});

describe("muted text and the primary button", () => {
  /** What muted text sits on: the page, a card, the sidebar, and a muted fill (a log label, a read-only field). */
  const UNDER_MUTED_TEXT = ["background", "card", "sidebar", "muted"] as const;

  test.each(THEMES)("%s: muted text reaches 4.5:1 on everything it sits on", (_theme, selector) => {
    const text = oklch(token(selector, "muted-foreground"));
    for (const surface of UNDER_MUTED_TEXT) {
      expect(contrast(text, oklch(token(selector, surface))), `${selector} muted text on ${surface}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  /** How opaque the default button's fill is while hovered: the `hover:bg-primary/NN` of components/ui/button.tsx. */
  function hoverAlpha(): number {
    const match = /default: "[^"]*\bhover:bg-primary\/(\d+)/.exec(buttonSource);
    expect(match, "the default button's hover fill").not.toBeNull();
    return Number(match![1]) / 100;
  }

  test.each(THEMES)("%s: the label of a hovered primary button reaches 4.5:1 on the page and on a card", (_theme, selector) => {
    const label = oklch(token(selector, "primary-foreground"));
    const alpha = hoverAlpha();
    for (const surface of ["background", "card"] as const) {
      const fill = blend(oklch(token(selector, "primary")), oklch(token(selector, surface)), alpha);
      expect(contrast(label, fill), `${selector} primary/${alpha * 100} over ${surface}`).toBeGreaterThanOrEqual(4.5);
    }
  });
});

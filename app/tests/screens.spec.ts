import { readFileSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";

// Screenshots of the panel's key screens in two languages and both themes, compared with the
// baselines in docs/gates/screenshots/m2/, and the layout checks that need a real browser (jsdom
// has no layout). Local only: see playwright.config.ts. Every screen shows the mock backend's
// fictional data; the address picks the scenario (?mock=) and the language (?lang=).

type Language = "zh-CN" | "zh-TW" | "en" | "ja" | "ko";
/** One of the panel's own texts, in one language, by its translation key. */
type Words = (key: string) => string;

/** The comparison allows 1% of a screen's pixels to differ (type rendering), and freezes animations. */
const SHOT = { maxDiffPixelRatio: 0.01, animations: "disabled" } as const;

const SCREENSHOT_LANGUAGES: Language[] = ["zh-CN", "en"];
/** English has the widest table; Chinese and Japanese are another script, without spaces between words. */
const LAYOUT_LANGUAGES: Language[] = ["en", "zh-CN", "ja"];
/** The smallest window the panel allows, and the same with the scroll bar Windows adds (15 px). */
const SMALLEST = [
  { width: 900, height: 600 },
  { width: 885, height: 600 },
];

const MESSAGES = new URL("../src/i18n/locales/", import.meta.url);
const known = new Map<Language, Words>();

/** The panel's words in `language`: the tests find controls by the text a person reads, not by a copy of it. */
function words(language: Language): Words {
  let t = known.get(language);
  if (t === undefined) {
    const messages: unknown = JSON.parse(readFileSync(new URL(`${language}.json`, MESSAGES), "utf8"));
    t = (key) => {
      let node = messages;
      for (const part of key.split(".")) {
        node = typeof node === "object" && node !== null ? (node as Record<string, unknown>)[part] : undefined;
      }
      if (typeof node !== "string") throw new Error(`there is no text "${key}" in ${language}.json`);
      return node;
    };
    known.set(language, t);
  }
  return t;
}

// A page that reports an error, or logs one, is not a page to take a picture of.
const errorsOf = new WeakMap<Page, string[]>();

test.beforeEach(({ page }) => {
  const errors: string[] = [];
  errorsOf.set(page, errors);
  page.on("pageerror", (error) => errors.push(`page error: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() !== "error") return;
    const { url } = message.location();
    // A browser asks for a favicon; the panel has none, as the window of the app has no use for one.
    if (url.endsWith("/favicon.ico")) return;
    errors.push(`console error: ${message.text()} ${url}`);
  });
});

test.afterEach(({ page }) => {
  expect(errorsOf.get(page), "what the page reported").toEqual([]);
});

/** Opens the panel at a hash route in `language`, on the mock's `scenario`. */
async function open(page: Page, language: Language, route: string, scenario: "demo" | "fresh" = "demo"): Promise<void> {
  await page.goto(`/?mock=${scenario}&lang=${language}#${route}`);
  // The panel starts in the system's language and changes to the one in its settings once it has read them.
  await expect(page.locator("html")).toHaveAttribute("lang", language);
}

/** The page's title, once it is on the screen. */
function title(page: Page, text: string): Locator {
  return page.getByRole("heading", { level: 1, name: text, exact: true });
}

/** Waits until nothing is loading any more and the page has been drawn. */
async function settled(page: Page): Promise<void> {
  await expect(page.locator('[data-slot="skeleton"]')).toHaveCount(0);
  await page.evaluate(async () => {
    await document.fonts.ready;
    // The app's icon is an image: drawn only once it has been decoded (a broken one fails here).
    await Promise.all([...document.images].map((image) => image.decode()));
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  });
}

/** Compares the page with its baseline, `<language>-<theme>-<name>.png`. */
async function shot(page: Page, language: Language, name: string): Promise<void> {
  const theme = test.info().project.name;
  // A click leaves the pointer where it was: put it where it hovers over nothing.
  await page.mouse.move(0, 0);
  await expect(page).toHaveScreenshot(`${language}-${theme}-${name}.png`, SHOT);
}

/** The eight screens of the panel's pages. `ready` waits for what shows the page has its data (the same in every language). */
const SCREENS: { name: string; route: string; ready: (page: Page, t: Words) => Promise<void> }[] = [
  {
    name: "overview",
    route: "/",
    ready: async (page, t) => {
      await expect(title(page, t("nav.overview"))).toBeVisible();
      await expect(page.getByRole("region", { name: t("overview.running") })).toContainText("DEMO-12");
      await expect(page.getByRole("region", { name: t("overview.recent") })).toContainText("DEMO-10");
    },
  },
  {
    name: "issues",
    route: "/issues",
    ready: async (page, t) => {
      await expect(title(page, t("nav.issues"))).toBeVisible();
      await expect(page.getByText("Update the README badges")).toBeVisible();
    },
  },
  {
    name: "runs",
    route: "/runs",
    ready: async (page, t) => {
      await expect(title(page, t("nav.runs"))).toBeVisible();
      await expect(page.getByText("executor exited with code 1")).toBeVisible();
    },
  },
  {
    // DEMO-12 is the run in progress in the mock; its log shows a session with a command.
    name: "run-detail",
    route: "/runs/12",
    ready: async (page) => {
      await expect(page.getByRole("heading", { level: 1 })).toContainText("DEMO-12");
      await expect(page.getByText("I will add the endpoint and a test for it.")).toBeVisible();
    },
  },
  {
    name: "bindings",
    route: "/bindings",
    ready: async (page, t) => {
      await expect(title(page, t("nav.bindings"))).toBeVisible();
      await expect(page.getByRole("region", { name: "Demo Mobile" })).toBeVisible();
    },
  },
  {
    name: "doctor",
    route: "/doctor",
    ready: async (page, t) => {
      await expect(title(page, t("nav.doctor"))).toBeVisible();
      await expect(page.getByText("codex-cli 0.159.2")).toBeVisible();
      await expect(page.getByRole("region", { name: "Demo Mobile" })).toBeVisible();
    },
  },
  {
    name: "settings",
    route: "/settings",
    ready: async (page, t) => {
      await expect(title(page, t("nav.settings"))).toBeVisible();
      await expect(page.getByRole("button", { name: t("common.save"), exact: true })).toBeVisible();
    },
  },
  {
    name: "help",
    route: "/help",
    ready: async (page, t) => {
      await expect(title(page, t("nav.help"))).toBeVisible();
    },
  },
];

/** The steps of the first-run guide, in order; the keys are those of `onboarding.title.*`. */
const GUIDE = ["language", "linear", "workspace", "executors", "binding", "finish"] as const;

/**
 * Walks the first-run guide on the mock's `fresh` scenario from its first step to its last, as a
 * person would, and calls `at` on each step (1 to 6) once the step has been drawn: Linear is
 * authorized on step 2, and step 5 is left with "Bind later".
 */
async function walkGuide(page: Page, language: Language, at: (step: number) => Promise<void>): Promise<void> {
  const t = words(language);
  const next = page.getByRole("button", { name: t("onboarding.next"), exact: true });
  const reached = async (step: number) => {
    await expect(title(page, t(`onboarding.title.${GUIDE[step - 1]}`))).toBeVisible();
    await settled(page);
    await at(step);
  };

  await open(page, language, "/onboarding", "fresh");
  await reached(1);
  await next.click();
  await reached(2);
  await page.getByRole("button", { name: t("onboarding.authorize"), exact: true }).click();
  // The person who authorized shows; only then can the guide go on.
  await expect(page.getByText("Jane Doe")).toBeVisible();
  await next.click();
  await reached(3);
  await next.click();
  // The executors are listed once the environment check has answered.
  await expect(page.getByRole("heading", { level: 3, name: "Claude Code" })).toBeVisible();
  await reached(4);
  await next.click();
  await reached(5);
  await page.getByRole("button", { name: t("onboarding.bind_later"), exact: true }).click();
  await expect(page.getByRole("button", { name: t("onboarding.start"), exact: true })).toBeEnabled();
  await reached(6);
}

for (const language of SCREENSHOT_LANGUAGES) {
  test.describe(`screenshots in ${language}`, () => {
    for (const screen of SCREENS) {
      test(screen.name, async ({ page }) => {
        await open(page, language, screen.route);
        await screen.ready(page, words(language));
        await settled(page);
        await shot(page, language, screen.name);
      });
    }

    // Steps 1, 2, 4 and 6 are pictured: step 3 is a check like step 4, and step 5 is the form of the Bindings page.
    test("onboarding", async ({ page }) => {
      await walkGuide(page, language, async (step) => {
        if ([1, 2, 4, 6].includes(step)) await shot(page, language, `onboarding-${step}`);
      });
    });
  });
}

// ---------------------------------------------------------------------------------------------
// Layout at the smallest window. These need a real browser: jsdom has no layout.

/** How far a container's content reaches past its right edge, and which buttons and links do. */
async function reachPastRightEdge(container: Locator): Promise<{ overflow: number; past: string[] }> {
  return container.evaluate((element) => {
    const right = element.getBoundingClientRect().right;
    const past = [...element.querySelectorAll<HTMLElement>("button, a[href]")]
      .filter((control) => control.getBoundingClientRect().right > right + 0.5)
      .map((control) => control.getAttribute("aria-label") ?? control.textContent?.trim() ?? control.tagName);
    return { overflow: element.scrollWidth - element.clientWidth, past };
  });
}

/** The element and everything in it is inside the window, not cut off by it or by a scrolling parent. */
async function expectWithinWindow(...elements: Locator[]): Promise<void> {
  for (const element of elements) {
    await expect(element).toBeVisible();
    await expect(element).toBeInViewport({ ratio: 1 });
  }
}

for (const { width, height } of SMALLEST) {
  test.describe(`at ${width}x${height}`, () => {
    test.use({ viewport: { width, height } });

    // The tables of issues and runs, with the buttons of their rows and the filter tabs: they used to run past the container.
    for (const language of LAYOUT_LANGUAGES) {
      for (const route of ["/issues", "/runs"]) {
        test(`${route} in ${language} does not overflow and keeps every button inside`, async ({ page }) => {
          const t = words(language);
          await open(page, language, route);
          await expect(title(page, t(route === "/issues" ? "nav.issues" : "nav.runs"))).toBeVisible();
          await expect(page.locator('[data-slot="table-container"]')).toBeVisible();
          await settled(page);

          expect(await reachPastRightEdge(page.locator('[data-slot="table-container"]')), "the table").toEqual({ overflow: 0, past: [] });
          expect(await reachPastRightEdge(page.locator("main")), "the page").toEqual({ overflow: 0, past: [] });
          const pastWindow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
          expect(pastWindow, "the window").toBeLessThanOrEqual(0);
        });
      }
    }

    // The dialog of a binding is no taller than the window: its fields scroll inside it and its buttons stay in sight.
    for (const language of ["en", "zh-CN"] as const) {
      for (const mode of ["add", "edit"] as const) {
        test(`the dialog to ${mode} a binding in ${language} fits the window with Cancel and Save`, async ({ page }) => {
          const t = words(language);
          await open(page, language, "/bindings");
          await expect(page.getByRole("region", { name: "Demo Project" })).toBeVisible();
          const opener =
            mode === "add"
              ? page.getByRole("button", { name: t("bindings.add"), exact: true })
              : page.getByRole("button", { name: t("bindings.edit"), exact: true }).first();
          await opener.click();
          const dialog = page.getByRole("dialog");
          await expect(dialog).toBeVisible();
          // Measured when it has finished opening: it grows into place.
          await dialog.evaluate(async (element) => {
            await Promise.all(element.getAnimations().map((animation) => animation.finished));
          });
          const cancel = dialog.getByRole("button", { name: t("common.cancel"), exact: true });
          const save = dialog.getByRole("button", { name: t("common.save"), exact: true });
          await expectWithinWindow(dialog, cancel, save);
        });
      }
    }

    // On every step of the guide the way out in the header and the Back and forward buttons are in reach.
    for (const language of ["en", "zh-CN"] as const) {
      test(`the guide in ${language} keeps its exit and its Back and forward buttons in reach`, async ({ page }) => {
        const t = words(language);
        await walkGuide(page, language, async () => {
          const buttons = page.locator("main footer").getByRole("button");
          const exit = page.getByRole("button", { name: t("onboarding.set_up_later"), exact: true });
          await expectWithinWindow(exit, buttons.first(), buttons.last());
        });
      });
    }

    // The settings' PATH row with its switch on, which adds the shell line and its Copy button: the switch, the
    // line and the button are in reach and nothing runs past the System card (a long line breaks, Copy moves down).
    for (const language of LAYOUT_LANGUAGES) {
      test(`the PATH row of the settings in ${language} keeps its switch, the shell line and Copy inside`, async ({ page }) => {
        const t = words(language);
        await open(page, language, "/settings");
        await expect(title(page, t("nav.settings"))).toBeVisible();
        // The switch is there once the state has been read.
        const toggle = page.getByRole("switch", { name: t("settings.cli.title"), exact: true });
        await toggle.click();
        const system = page.getByRole("region", { name: t("settings.group.system"), exact: true });
        const line = system.locator("code", { hasText: "export PATH" });
        await expect(line).toBeVisible();
        await settled(page);
        await system.scrollIntoViewIfNeeded();
        await expectWithinWindow(toggle, line, system.getByRole("button", { name: t("common.copy"), exact: true }));
        expect(await reachPastRightEdge(system), "the System card").toEqual({ overflow: 0, past: [] });
        expect(await reachPastRightEdge(page.locator("main")), "the page").toEqual({ overflow: 0, past: [] });
        const pastWindow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
        expect(pastWindow, "the window").toBeLessThanOrEqual(0);
      });
    }
  });
}

// Renders the IssueRelay icons from the SVG sources in app/icons-src/ (design 9.6).
//
// Usage (paths are relative to the repository root):
//   pnpm --dir app icons --preview          draws the three options side by side:
//                                             docs/design/icons/preview.png
//   pnpm --dir app icons --option <a|b|c>   writes every icon of one option:
//     app/icons-src/build/app-1024.png            1024 px app icon, the input of `tauri icon` (git-ignored)
//     app/src-tauri/icons/tray/macos/<state>.png  macOS template images, 44 px, black plus alpha only
//     app/src-tauri/icons/tray/<state>.png        Windows colour tray icons, 32 px
//     app/public/icon.svg                         the app icon without its transparent margin
//   Add `--out <dir>` to write the same tree under <dir> instead of the repository, e.g. to try a
//   render without touching the committed icons. <state> is idle, running, paused or problem.
//
// Exit codes: 0 done; 1 a source file is missing or a rendered image failed its check; 2 bad arguments.
//
// Source conventions (app/icons-src/option-<x>/):
//   app.svg         1024 x 1024; the macOS grid: an 824 px rounded square centred on the canvas. Its
//                   drop shadow is the filter `lift`, which `public/icon.svg` leaves out.
//   tray-mono.svg   32 x 32 grid; the ink is `currentColor` (the root sets `color="#000"`), so it
//                   renders black plus alpha and the preview can tint it for a dark menu bar.
//   tray-color.svg  32 x 32 grid; the same glyph in colour.
//   Both tray glyphs keep the bottom-right corner free: this script draws the status badge there
//   (the SVGs have none), after cutting a gap into the glyph around it.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { parseArgs } from "node:util";
import { Resvg } from "@resvg/resvg-js";

const appDir = path.resolve(import.meta.dirname, "..");
const repoRoot = path.resolve(appDir, "..");
const sourceDir = path.join(appDir, "icons-src");

/** What each option is about; the preview prints it next to its row. */
const OPTIONS = {
  a: { name: "Relay", about: ["Tilted baton with speed", "lines trailing behind it."] },
  b: { name: "Handover", about: ["Two dots joined by an arc", "that ends in an arrow."] },
  c: { name: "Ticket + start", about: ["An issue card with a play", "triangle: run this one."] },
};
const STATES = ["idle", "running", "paused", "problem"];

const APP_SIZE = 1024;
// The rounded square sits in the middle of the canvas; `public/icon.svg` is cropped to it.
const PLATE_MARGIN = 100;
// tray-icon 0.25.1 shows any macOS tray image 18 pt high, so 44 px is a little over 2x on a Retina
// menu bar. Windows gets the 32 px of a 200% display; the system scales it down for 100%.
const MAC_TRAY = 44;
const WIN_TRAY = 32;

// Tray glyphs are drawn on a 32 unit grid; the badge is centred on 24.5 on both axes.
const GRID = 32;
const BADGE_CENTRE = 24.5;
// Radius of the gap cut into the glyph around the badge, so the badge never touches it.
const MONO_GAP = 8.4;
const COLOUR_GAP = 7.8;
// Windows badge colours: the panel's running blue, failed red and a neutral grey, in the same hues
// but lighter, so they also show on a dark taskbar. The marks on them are white.
const BADGE_COLOUR = { running: "#1d84f5", paused: "#83868c", problem: "#ee343b" };

/** The status mark of a macOS template image: ink only, no disc around it. */
const MONO_MARKS = {
  running: `<circle cx="${BADGE_CENTRE}" cy="${BADGE_CENTRE}" r="5.2" fill="currentColor"/>`,
  paused:
    `<rect x="19.8" y="19.4" width="3.6" height="10.2" rx="0.9" fill="currentColor"/>` +
    `<rect x="25.6" y="19.4" width="3.6" height="10.2" rx="0.9" fill="currentColor"/>`,
  problem:
    `<rect x="22.85" y="18" width="3.3" height="7.4" rx="1.1" fill="currentColor"/>` +
    `<circle cx="${BADGE_CENTRE}" cy="29.15" r="1.95" fill="currentColor"/>`,
};

/** The status mark of a Windows tray icon: a coloured disc with a white mark on it. */
const COLOUR_MARKS = {
  running: `<circle cx="${BADGE_CENTRE}" cy="${BADGE_CENTRE}" r="6" fill="${BADGE_COLOUR.running}"/>`,
  paused:
    `<circle cx="${BADGE_CENTRE}" cy="${BADGE_CENTRE}" r="6" fill="${BADGE_COLOUR.paused}"/>` +
    `<rect x="21.6" y="21.4" width="2.2" height="6.2" rx="0.6" fill="#fff"/>` +
    `<rect x="25.2" y="21.4" width="2.2" height="6.2" rx="0.6" fill="#fff"/>`,
  problem:
    `<circle cx="${BADGE_CENTRE}" cy="${BADGE_CENTRE}" r="6" fill="${BADGE_COLOUR.problem}"/>` +
    `<rect x="23.4" y="20.3" width="2.2" height="4.9" rx="1" fill="#fff"/>` +
    `<circle cx="${BADGE_CENTRE}" cy="27.3" r="1.2" fill="#fff"/>`,
};

/** A problem with the sources or the rendered images, reported without a stack trace. */
class CheckError extends Error {}

/** Renders `svg` to a PNG `size` px wide. `pixels` is the raw RGBA data, for the checks. */
function render(svg, size) {
  const image = new Resvg(svg, { fitTo: { mode: "width", value: size } }).render();
  return { png: image.asPng(), pixels: image.pixels, width: image.width };
}

/** The three source files of an option, read and checked against the source conventions. */
function loadOption(letter) {
  const read = (file, viewBox) => {
    const name = `option-${letter}/${file}`;
    let svg;
    try {
      svg = readFileSync(path.join(sourceDir, name), "utf8");
    } catch (error) {
      throw new CheckError(`cannot read app/icons-src/${name} (${error.code ?? error.message})`);
    }
    if (!svg.includes(`viewBox="${viewBox}"`)) throw new CheckError(`${name} must be drawn on viewBox="${viewBox}"`);
    return svg;
  };
  return {
    app: read("app.svg", `0 0 ${APP_SIZE} ${APP_SIZE}`),
    mono: read("tray-mono.svg", `0 0 ${GRID} ${GRID}`),
    colour: read("tray-color.svg", `0 0 ${GRID} ${GRID}`),
  };
}

/** Everything between the root tags of an SVG document. */
function innerMarkup(svg) {
  const match = /<svg\b[^>]*>([\s\S]*)<\/svg>\s*$/.exec(svg);
  if (match === null) throw new CheckError("a source file is not an SVG document");
  return match[1];
}

/**
 * One tray icon: the glyph of `kind` ("mono" or "colour") with the badge of `state` in its
 * bottom-right corner. `ink` is the colour of a mono glyph and its badge: the template's black, or
 * white to show how the system tints it on a dark menu bar.
 */
function traySvg(glyph, kind, state, size, ink = "#000") {
  const head = `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${GRID} ${GRID}" color="${ink}">`;
  const body = innerMarkup(glyph);
  if (state === "idle") return `${head}${body}</svg>`;
  const gap = kind === "mono" ? MONO_GAP : COLOUR_GAP;
  const marks = (kind === "mono" ? MONO_MARKS : COLOUR_MARKS)[state];
  const mask =
    `<mask id="badge-gap" maskUnits="userSpaceOnUse" x="0" y="0" width="${GRID}" height="${GRID}">` +
    `<rect width="${GRID}" height="${GRID}" fill="#fff"/>` +
    `<circle cx="${BADGE_CENTRE}" cy="${BADGE_CENTRE}" r="${gap}" fill="#000"/></mask>`;
  return `${head}<defs>${mask}</defs><g mask="url(#badge-gap)">${body}</g>${marks}</svg>`;
}

const trayImage = (option, kind, state, size, ink) =>
  render(traySvg(kind === "mono" ? option.mono : option.colour, kind, state, size, ink), size);

/**
 * The app icon without its margin, for the panel (the sidebar and the first-run header). The
 * margin is where the plate's drop shadow (the filter `lift`) falls; cropped to the plate it would
 * only smudge the corners, so the shadow goes with the margin.
 */
function panelIconSvg(appSvg) {
  const plate = APP_SIZE - 2 * PLATE_MARGIN;
  const rootTag = /<svg\b[^>]*>/.exec(appSvg)?.[0];
  if (rootTag === undefined) throw new CheckError("app.svg has no root element");
  const cropped = rootTag
    .replace(/\bwidth="[^"]*"/, `width="${plate}"`)
    .replace(/\bheight="[^"]*"/, `height="${plate}"`)
    .replace(/\bviewBox="[^"]*"/, `viewBox="${PLATE_MARGIN} ${PLATE_MARGIN} ${plate} ${plate}"`);
  const flat = appSvg
    .replace(rootTag, cropped)
    .replace(/ filter="url\(#lift\)"/, "")
    .replace(/\n? *<filter id="lift"[\s\S]*?<\/filter>/, "");
  if (/#lift|id="lift"/.test(flat)) {
    throw new CheckError("app.svg: the plate's drop shadow must be one filter `lift`, used once, on the plate");
  }
  return flat;
}

function writeOutput(outRoot, relative, data) {
  const file = path.join(outRoot, relative);
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, data);
  console.log(`wrote ${relative}`);
}

/** Fails if a rendered icon is not what its source promises, before anything is written. */
function checkImages(letter, images) {
  const fail = (what) => {
    throw new CheckError(`option ${letter}: ${what}`);
  };
  for (const state of STATES) {
    // A template image is black plus alpha; the RGB of black ink is 0 whether or not the pixels
    // are premultiplied, so any other value means a colour or a grey got in.
    const { pixels } = images.mac[state];
    for (let i = 0; i < pixels.length; i += 4) {
      if (pixels[i] !== 0 || pixels[i + 1] !== 0 || pixels[i + 2] !== 0) {
        fail(`the macOS ${state} image has a pixel that is not black`);
      }
    }
    if (!pixels.some((value, i) => i % 4 === 3 && value > 0)) fail(`the macOS ${state} image is empty`);
  }
  for (const set of [images.mac, images.win]) {
    STATES.forEach((state, i) => {
      for (const other of STATES.slice(i + 1)) {
        if (set[state].png.equals(set[other].png)) fail(`the ${state} and ${other} tray images are identical`);
      }
    });
  }
  const { pixels, width } = images.app;
  const alphaAt = (x, y) => pixels[(y * width + x) * 4 + 3];
  const last = width - 1;
  const corners = [alphaAt(0, 0), alphaAt(last, 0), alphaAt(0, last), alphaAt(last, last)];
  if (corners.some((alpha) => alpha !== 0) || alphaAt(width / 2, width / 2) !== 255) {
    fail("the app icon does not sit on the 1024 grid (transparent corners, opaque centre)");
  }
}

/** `--option`: every file of one option. */
function buildOption(letter, outRoot) {
  const option = loadOption(letter);
  const images = {
    app: render(option.app, APP_SIZE),
    mac: Object.fromEntries(STATES.map((state) => [state, trayImage(option, "mono", state, MAC_TRAY)])),
    win: Object.fromEntries(STATES.map((state) => [state, trayImage(option, "colour", state, WIN_TRAY)])),
  };
  checkImages(letter, images);
  writeOutput(outRoot, "app/icons-src/build/app-1024.png", images.app.png);
  for (const state of STATES) {
    writeOutput(outRoot, `app/src-tauri/icons/tray/macos/${state}.png`, images.mac[state].png);
    writeOutput(outRoot, `app/src-tauri/icons/tray/${state}.png`, images.win[state].png);
  }
  writeOutput(outRoot, "app/public/icon.svg", panelIconSvg(option.app));
  console.log("next: pnpm --dir app tauri icon icons-src/build/app-1024.png");
}

// ---- The comparison sheet ---------------------------------------------------------------------
//
// One row per option, left to right: the label; the app icon (the 1024 px master scaled to fit,
// then 128, 32 and 16 px); the tray states on a light and a dark bar. Every raster is rendered at
// the size it is shown, so the small ones are what a platform really draws. The enlarged copies
// repeat pixels instead of smoothing them, to show what is in a 16 px icon.

const SHEET_WIDTH = 1500;
const MARGIN = 24;
const HEADER_HEIGHT = 108;
const ROW_HEIGHT = 300;
const ROW_GAP = 14;
const FONT = `font-family="Helvetica Neue, Helvetica, Arial, sans-serif"`;
const COLOURS = {
  page: "#e8e8ee",
  card: "#ffffff",
  cardLine: "#d3d3dd",
  lightTile: "#f1f1f6",
  darkTile: "#1f2025",
  lightBar: "#f3f3f6",
  darkBar: "#25262b",
  text: "#1b1b27",
  muted: "#5a5a70",
  mutedOnDark: "#b9b9c6",
  accent: "#4a4edb",
};

const escapeXml = (text) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const label = (x, y, content, { size = 14, weight = 400, fill = COLOURS.muted, anchor = "start" } = {}) =>
  `<text x="${x}" y="${y}" ${FONT} font-size="${size}" font-weight="${weight}" fill="${fill}" text-anchor="${anchor}">${escapeXml(content)}</text>`;

const box = (x, y, width, height, fill, { radius = 10, stroke = "none" } = {}) =>
  `<rect x="${x}" y="${y}" width="${width}" height="${height}" rx="${radius}" fill="${fill}" stroke="${stroke}"/>`;

/** A rendered PNG placed at `x`, `y`, `size` px wide. `crisp` enlarges by repeating pixels. */
const picture = (image, x, y, size, crisp = false) =>
  `<image x="${x}" y="${y}" width="${size}" height="${size}" href="data:image/png;base64,${image.png.toString("base64")}"` +
  `${crisp ? ` image-rendering="optimizeSpeed"` : ""}/>`;

/** The letter, the name and the one-line idea of an option. */
function rowLabel(letter, y) {
  const { name, about } = OPTIONS[letter];
  const x = MARGIN + 24;
  return [
    label(x, y + 78, letter.toUpperCase(), { size: 68, weight: 700, fill: COLOURS.accent }),
    label(x, y + 116, name, { size: 24, weight: 700, fill: COLOURS.text }),
    ...about.map((line, i) => label(x, y + 144 + i * 20, line)),
  ];
}

/** The app icon: the master, then the sizes the platforms use, enlarged and at their own size. */
function rowAppIcons(option, top) {
  const app = (size) => render(option.app, size);
  const small32 = app(32);
  const small16 = app(16);
  const centred = { size: 12, anchor: "middle" };
  const parts = [
    box(234, top, 276, 268, COLOURS.lightTile),
    picture(app(240), 252, top + 6, 240),
    label(372, top + 258, "1024 px master, shown at 23%", centred),

    box(528, top, 148, 148, COLOURS.lightTile),
    picture(app(128), 538, top + 10, 128),
    label(602, top + 142, "128 px", centred),

    box(688, top, 148, 148, COLOURS.lightTile),
    picture(small32, 698, top + 10, 128, true),
    label(762, top + 142, "32 px, enlarged x4", centred),

    box(848, top, 148, 148, COLOURS.lightTile),
    picture(small16, 858, top + 10, 128, true),
    label(922, top + 142, "16 px, enlarged x8", centred),
  ];
  for (const [x, tile] of [[528, COLOURS.lightTile], [772, COLOURS.darkTile]]) {
    const caption = { ...centred, fill: tile === COLOURS.lightTile ? COLOURS.muted : COLOURS.mutedOnDark };
    parts.push(
      box(x, top + 160, 224, 108, tile),
      picture(small32, x + 56, top + 186, 32),
      picture(small16, x + 136, top + 194, 16),
      label(x + 72, top + 242, "32 px", caption),
      label(x + 144, top + 242, "16 px", caption),
    );
  }
  return parts;
}

/**
 * The tray states, on a light and on a dark bar. A cell holds the macOS template (44 px, tinted
 * black or white the way the system does) and the Windows colour icon (32 px), and below them both
 * at 16 px, enlarged x3.
 */
function rowTray(option, top) {
  const panelX = 1012;
  const panelWidth = SHEET_WIDTH - MARGIN - 14 - panelX;
  const cellWidth = (panelWidth - 16) / STATES.length;
  const parts = [];
  for (const [index, dark] of [false, true].entries()) {
    const panelY = top + index * 138;
    const ink = dark ? "#ffffff" : "#000000";
    parts.push(box(panelX, panelY, panelWidth, 130, dark ? COLOURS.darkBar : COLOURS.lightBar));
    STATES.forEach((state, i) => {
      const x = panelX + 12 + i * cellWidth;
      parts.push(
        picture(trayImage(option, "mono", state, MAC_TRAY, ink), x, panelY + 10, MAC_TRAY),
        picture(trayImage(option, "colour", state, WIN_TRAY), x + 56, panelY + 16, WIN_TRAY),
        picture(trayImage(option, "mono", state, 16, ink), x, panelY + 62, 48, true),
        picture(trayImage(option, "colour", state, 16), x + 56, panelY + 62, 48, true),
        label(x, panelY + 124, state, { size: 12, fill: dark ? COLOURS.mutedOnDark : COLOURS.muted }),
      );
    });
  }
  return parts;
}

function sheetRow(letter, y) {
  const option = loadOption(letter);
  const top = y + 16;
  return [
    box(MARGIN, y, SHEET_WIDTH - 2 * MARGIN, ROW_HEIGHT, COLOURS.card, { radius: 16, stroke: COLOURS.cardLine }),
    ...rowLabel(letter, y),
    ...rowAppIcons(option, top),
    ...rowTray(option, top),
  ];
}

/** `--preview`: the three options on one sheet, one row each. */
function buildPreview(outRoot) {
  const letters = Object.keys(OPTIONS);
  const height = HEADER_HEIGHT + letters.length * (ROW_HEIGHT + ROW_GAP) + MARGIN - ROW_GAP;
  const parts = [
    box(0, 0, SHEET_WIDTH, height, COLOURS.page, { radius: 0 }),
    label(MARGIN, 46, "IssueRelay icon options", { size: 28, weight: 700, fill: COLOURS.text }),
    label(
      MARGIN,
      72,
      "Left to right: the app icon (1024 px master, then 128, 32 and 16 px), and the four tray states idle, running, paused, problem on a light and a dark bar.",
    ),
    label(
      MARGIN,
      92,
      "Each tray cell: top left = macOS template (44 px, black plus alpha, tinted by the system); top right = Windows colour (32 px); below them both at 16 px, enlarged x3.",
    ),
  ];
  letters.forEach((letter, i) => parts.push(...sheetRow(letter, HEADER_HEIGHT + i * (ROW_HEIGHT + ROW_GAP))));
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${SHEET_WIDTH}" height="${height}" viewBox="0 0 ${SHEET_WIDTH} ${height}">${parts.join("")}</svg>`;
  const sheet = new Resvg(svg, { font: { loadSystemFonts: true } }).render();
  writeOutput(outRoot, "docs/design/icons/preview.png", sheet.asPng());
}

// ---- Command line -----------------------------------------------------------------------------

const USAGE = "usage: node scripts/icons.mjs [--preview] [--option <a|b|c>] [--out <dir>]";

function parseArguments() {
  try {
    const { values } = parseArgs({
      options: {
        preview: { type: "boolean" },
        option: { type: "string" },
        out: { type: "string" },
        help: { type: "boolean", short: "h" },
      },
      strict: true,
    });
    return values;
  } catch (error) {
    return { error: error.message };
  }
}

function main() {
  const args = parseArguments();
  if (args.help) {
    console.log(USAGE);
    return 0;
  }
  if (args.error !== undefined || (!args.preview && args.option === undefined)) {
    console.error(`icons: ${args.error ?? "nothing to do"}\n${USAGE}`);
    return 2;
  }
  const letter = args.option?.toLowerCase();
  if (letter !== undefined && !(letter in OPTIONS)) {
    console.error(`icons: unknown option "${args.option}" (expected a, b or c)\n${USAGE}`);
    return 2;
  }
  const outRoot = args.out === undefined ? repoRoot : path.resolve(args.out);
  try {
    if (args.preview) buildPreview(outRoot);
    if (letter !== undefined) buildOption(letter, outRoot);
    return 0;
  } catch (error) {
    if (!(error instanceof CheckError)) throw error;
    console.error(`icons: ${error.message}`);
    return 1;
  }
}

process.exitCode = main();

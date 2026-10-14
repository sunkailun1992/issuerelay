#!/usr/bin/env node
// Release gates: checks a release must pass, as pure functions over what they are given, so that
// `self-test` checks every gate, failing cases included, before any of them runs for real. The
// release workflow (.github/workflows/release.yml, bundle.yml) runs them for a tag and, in a dry
// run, for the tag that would be pushed and the dry run's bundles; developers run `panel` before
// bundling on their own machine.
//
// Usage: node scripts/release-checks.mjs <gate> [options]
//   panel [--index <file>]   the panel about to be embedded (app/dist/index.html by default) is the
//                            real one, not the placeholder page app/src-tauri/build.rs writes when
//                            the panel was not built
//   self-test                every gate against made-up input
// Exit codes: 0 = passed, 1 = a gate failed, 2 = bad arguments or unreadable input.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// ---- gates ---------------------------------------------------------------------------------------

/** A sentence only the placeholder page of app/src-tauri/build.rs has (`self-test` checks build.rs still writes it). */
export const PLACEHOLDER_MARKER = "to build the panel.";

/**
 * What is wrong with the panel page about to be embedded into the app: missing, or the placeholder
 * build.rs writes so that a build without `pnpm build` still compiles. `html` is null when the file
 * does not exist. A release build embeds app/dist as it is, so the placeholder would ship.
 */
export function panelProblems(html) {
  if (html === null) return ["app/dist/index.html is missing: run `pnpm --dir app build` first"];
  if (html.includes(PLACEHOLDER_MARKER)) {
    return ["app/dist/index.html is build.rs's placeholder page, not the panel: run `pnpm --dir app build` first"];
  }
  return [];
}

// ---- self-test -----------------------------------------------------------------------------------

/** The line of build.rs that holds its placeholder page: the gate is tested against the real thing. */
function placeholderPage() {
  const buildRs = readFileSync(path.join(root, "app/src-tauri/build.rs"), "utf8");
  const line = buildRs.split("\n").find((text) => text.includes(PLACEHOLDER_MARKER));
  assert.ok(line, `build.rs no longer writes "${PLACEHOLDER_MARKER}": update PLACEHOLDER_MARKER with it`);
  return line;
}

const SELF_TESTS = {
  panel() {
    const built = '<!doctype html><html><head><title>IssueRelay</title><script type="module" src="/assets/index.js"></script></head><body><div id="root"></div></body></html>';
    assert.deepEqual(panelProblems(built), []);
    assert.match(panelProblems(null).join(), /is missing/);
    assert.match(panelProblems(placeholderPage()).join(), /placeholder/);
  },
};

function selfTest() {
  let failed = 0;
  for (const [name, test] of Object.entries(SELF_TESTS)) {
    try {
      test();
      console.log(`ok ${name}`);
    } catch (error) {
      failed += 1;
      console.log(`FAILED ${name}: ${error.message}`);
    }
  }
  return failed === 0 ? 0 : 1;
}

// ---- command line --------------------------------------------------------------------------------

/** Each gate's options, and what it does with them: the list of problems it found. */
const GATES = {
  panel: {
    options: { index: { type: "string" } },
    run: (values) => {
      const file = path.resolve(values.index ?? path.join(root, "app/dist/index.html"));
      return panelProblems(existsSync(file) ? readFileSync(file, "utf8") : null);
    },
  },
};

function main(argv) {
  const [name, ...rest] = argv;
  if (name === "self-test" && rest.length === 0) return selfTest();
  const gate = GATES[name];
  if (gate === undefined) throw new Error(`usage: node scripts/release-checks.mjs <${[...Object.keys(GATES), "self-test"].join("|")}> [options]`);
  const { values } = parseArgs({ args: rest, options: gate.options, strict: true });
  const problems = gate.run(values);
  for (const problem of problems) console.log(`FAILED ${name}: ${problem}`);
  if (problems.length === 0) console.log(`ok ${name}`);
  return problems.length === 0 ? 0 : 1;
}

try {
  process.exitCode = main(process.argv.slice(2));
} catch (error) {
  console.error(`release-checks: ${error.message}`);
  process.exitCode = 2;
}

#!/usr/bin/env node
// Release gates: checks a release must pass, as pure functions over what they are given, so that
// `self-test` checks every gate, failing cases included, before any of them runs for real. The
// release workflow (.github/workflows/release.yml, bundle.yml) runs them for a tag and, in a dry
// run, for the tag that would be pushed and the dry run's bundles; developers run `panel` before
// bundling on their own machine.
//
// Usage: node scripts/release-checks.mjs <gate> [options]
//   panel [--index <file>]          the panel about to be embedded (app/dist/index.html by default)
//                                   is the real one, not build.rs's placeholder page
//   tag --tag <tag> [--manifest <Cargo.toml>]
//                                   the tag names the workspace version, which is X.Y.Z
//   on-main --commit <sha> [--ref <ref>]
//                                   the commit is on the ref (origin/main by default)
//   assets --version <X.Y.Z> --updater true|false --mode release|dry-run --names <file>
//                                   every asset the release (or the dry run) must have is listed
//                                   in <file>, one name per line
//   latest-json --version <X.Y.Z> --latest <file> --assets <file>
//                                   latest.json announces the version, and every platform key has
//                                   a signature and points at the right asset of the release
//                                   (<file>: one "<id><TAB><name>" line per asset)
//   latest-json-dry-run --version <X.Y.Z> --dir <dir>
//                                   the same for the latest.json a release of the dry run's
//                                   bundles in <dir> would get
//   workspace-version               prints the version in the root Cargo.toml (not a gate)
//   self-test                       every gate against made-up input, passing and failing cases
// Exit codes: 0 = passed, 1 = a gate failed, 2 = bad arguments or unreadable input.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
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

/** The workspace version: `version` in the `[workspace.package]` table of the root Cargo.toml. */
export function workspaceVersion(cargoToml) {
  const table = cargoToml.split(/^\[/m).find((part) => part.startsWith("workspace.package]"));
  const match = table?.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) throw new Error("Cargo.toml has no [workspace.package] version");
  return match[1];
}

/** What is wrong with releasing `version` under `tag`. */
export function tagProblems(tag, version) {
  const problems = [];
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    problems.push(`the workspace version ${version} is not X.Y.Z (MSI refuses pre-release suffixes like -rc.1)`);
  }
  if (tag !== `v${version}`) problems.push(`tag ${tag} does not name the workspace version ${version} in Cargo.toml (expected v${version})`);
  return problems;
}

/** Whether `commit` is on `ref` (it, or one of its ancestors), as git says. */
export function onMainProblems(commit, ref, cwd = root) {
  try {
    execFileSync("git", ["merge-base", "--is-ancestor", commit, ref], { cwd, stdio: "ignore" });
    return [];
  } catch (error) {
    if (error.status === 1) return [`${commit} is not on ${ref}: a release is built only from a commit of main`];
    throw new Error(`git merge-base could not compare ${commit} with ${ref}`);
  }
}

/**
 * The assets a release (`mode` "release") or a dry run's bundles ("dry-run") of `version` must
 * have. tauri-action renames the macOS update payload when it uploads it; in a dry run it keeps
 * the bundler's own name. (Names from tauri-action v1.0.0 and tauri-bundler 2.10.2; the first
 * dry run shows the real ones.)
 */
export function expectedAssets(version, updater, mode) {
  const names = [`IssueRelay_${version}_universal.dmg`, `IssueRelay_${version}_x64-setup.exe`, `IssueRelay_${version}_x64_en-US.msi`];
  if (!updater) return names;
  const payload = mode === "release" ? `IssueRelay_${version}_universal.app.tar.gz` : "IssueRelay.app.tar.gz";
  names.push(payload, `${payload}.sig`, `IssueRelay_${version}_x64-setup.exe.sig`, `IssueRelay_${version}_x64_en-US.msi.sig`);
  if (mode === "release") names.push("latest.json");
  return names;
}

export function assetProblems(names, version, updater, mode) {
  const present = new Set(names);
  return expectedAssets(version, updater, mode)
    .filter((name) => !present.has(name))
    .map((name) => `missing asset ${name}`);
}

/**
 * The platform keys tauri-action v1.0.0 writes into latest.json for a universal macOS build and
 * the Windows installers (with `updaterJsonPreferNsis`), and the payload each must point at.
 */
export const PLATFORM_KEYS = {
  "darwin-aarch64": ".app.tar.gz",
  "darwin-aarch64-app": ".app.tar.gz",
  "darwin-x86_64": ".app.tar.gz",
  "darwin-x86_64-app": ".app.tar.gz",
  "darwin-universal": ".app.tar.gz",
  "darwin-universal-app": ".app.tar.gz",
  "windows-x86_64": "-setup.exe",
  "windows-x86_64-nsis": "-setup.exe",
  "windows-x86_64-msi": ".msi",
};

/** The asset a latest.json `url` names: GitHub's API asset URL ends in its id, a download URL (or a dry run's) in its name. */
function assetAt(url, assets) {
  const last = String(url ?? "").split("/").pop();
  return assets.find((asset) => String(asset.id) === last || asset.name === last);
}

/** What is wrong with `latest` for `version`, given the release's assets (`{ id, name }`). */
export function latestJsonProblems(latest, version, assets) {
  const problems = [];
  if (latest?.version !== version && latest?.version !== `v${version}`) problems.push(`latest.json announces ${latest?.version}, not ${version}`);
  for (const [key, suffix] of Object.entries(PLATFORM_KEYS)) {
    const entry = latest?.platforms?.[key];
    if (!entry) {
      problems.push(`latest.json has no entry for ${key}`);
      continue;
    }
    if (typeof entry.signature !== "string" || entry.signature.trim() === "") problems.push(`latest.json's ${key} has no signature`);
    const asset = assetAt(entry.url, assets);
    if (!asset) problems.push(`latest.json's ${key} points at ${entry.url}, which is no asset of this release`);
    else if (!asset.name.endsWith(suffix)) problems.push(`latest.json's ${key} points at ${asset.name}, not the ${suffix} payload`);
  }
  return problems;
}

/** The latest.json a release of these bundles would get, the way tauri-action writes it, with the bundle names as urls. */
export function wouldBeLatestJson(version, assets, signatureOf) {
  const platforms = {};
  for (const [key, suffix] of Object.entries(PLATFORM_KEYS)) {
    const asset = assets.find((candidate) => candidate.name.endsWith(suffix));
    if (asset) platforms[key] = { signature: signatureOf(asset.name), url: asset.name };
  }
  return { version, platforms };
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
  tag() {
    const manifest = '[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "0.2.0"\nedition = "2024"\n';
    assert.equal(workspaceVersion(manifest), "0.2.0");
    assert.throws(() => workspaceVersion('[package]\nversion = "0.2.0"\n'), /no \[workspace\.package\] version/);
    assert.deepEqual(tagProblems("v0.2.0", "0.2.0"), []);
    assert.match(tagProblems("v0.2.1", "0.2.0").join(), /does not name the workspace version 0\.2\.0/);
    assert.match(tagProblems("0.2.0", "0.2.0").join(), /expected v0\.2\.0/);
    assert.match(tagProblems("v0.2.0-rc.1", "0.2.0-rc.1").join(), /not X\.Y\.Z/);
    // The repository's own Cargo.toml reads.
    assert.match(workspaceVersion(readFileSync(path.join(root, "Cargo.toml"), "utf8")), /^\d+\.\d+\.\d+$/);
  },
  "on-main"() {
    const repo = mkdtempSync(path.join(tmpdir(), "release-checks-"));
    try {
      const git = (...args) =>
        execFileSync("git", ["-c", "user.name=Demo", "-c", "user.email=demo@example.com", "-c", "commit.gpgsign=false", ...args], {
          cwd: repo,
          stdio: "pipe",
        })
          .toString()
          .trim();
      git("init", "--quiet", "--initial-branch=main");
      git("commit", "--quiet", "--allow-empty", "-m", "one");
      const onMain = git("rev-parse", "HEAD");
      git("switch", "--quiet", "-c", "side");
      git("commit", "--quiet", "--allow-empty", "-m", "side work");
      const offMain = git("rev-parse", "HEAD");
      assert.deepEqual(onMainProblems(onMain, "main", repo), []);
      assert.match(onMainProblems(offMain, "main", repo).join(), /is not on main/);
    } finally {
      rmSync(repo, { recursive: true, force: true });
    }
  },
  assets() {
    const release = expectedAssets("0.2.0", true, "release");
    assert.deepEqual(assetProblems(release, "0.2.0", true, "release"), []);
    assert.deepEqual(assetProblems(release.filter((name) => !name.endsWith(".msi")), "0.2.0", true, "release"), [
      "missing asset IssueRelay_0.2.0_x64_en-US.msi",
    ]);
    // Bundles of another version are none of this version's assets. Only latest.json has no version
    // in its name, so it is not missing here; the `latest-json` gate checks which version it announces.
    assert.deepEqual(
      assetProblems(expectedAssets("0.1.9", true, "release"), "0.2.0", true, "release"),
      release.filter((name) => name !== "latest.json").map((name) => `missing asset ${name}`),
    );
    // Without the update key: no payloads, no signatures, no latest.json.
    assert.deepEqual(expectedAssets("0.2.0", false, "release"), [
      "IssueRelay_0.2.0_universal.dmg",
      "IssueRelay_0.2.0_x64-setup.exe",
      "IssueRelay_0.2.0_x64_en-US.msi",
    ]);
    // A dry run keeps the bundler's own name of the macOS payload and has no latest.json.
    assert.ok(expectedAssets("0.2.0", true, "dry-run").includes("IssueRelay.app.tar.gz.sig"));
    assert.ok(!expectedAssets("0.2.0", true, "dry-run").includes("latest.json"));
  },
  "latest-json"() {
    const assets = [
      { id: "11", name: "IssueRelay_0.2.0_universal.app.tar.gz" },
      { id: "22", name: "IssueRelay_0.2.0_x64-setup.exe" },
      { id: "33", name: "IssueRelay_0.2.0_x64_en-US.msi" },
    ];
    const api = (id) => `https://api.github.com/repos/demo/issuerelay/releases/assets/${id}`;
    const entry = (id) => ({ signature: "c2lnbmF0dXJl", url: api(id) });
    const idFor = { ".app.tar.gz": "11", "-setup.exe": "22", ".msi": "33" };
    const good = {
      version: "0.2.0",
      platforms: Object.fromEntries(Object.entries(PLATFORM_KEYS).map(([key, suffix]) => [key, entry(idFor[suffix])])),
    };
    assert.deepEqual(latestJsonProblems(good, "0.2.0", assets), []);
    assert.match(latestJsonProblems({ ...good, version: "0.1.9" }, "0.2.0", assets).join(), /announces 0\.1\.9, not 0\.2\.0/);
    const withoutUniversal = { ...good, platforms: { ...good.platforms } };
    delete withoutUniversal.platforms["darwin-universal"];
    assert.match(latestJsonProblems(withoutUniversal, "0.2.0", assets).join(), /no entry for darwin-universal/);
    const unsigned = { ...good, platforms: { ...good.platforms, "windows-x86_64-msi": { signature: "", url: api("33") } } };
    assert.match(latestJsonProblems(unsigned, "0.2.0", assets).join(), /windows-x86_64-msi has no signature/);
    const elsewhere = { ...good, platforms: { ...good.platforms, "darwin-aarch64": entry("99") } };
    assert.match(latestJsonProblems(elsewhere, "0.2.0", assets).join(), /no asset of this release/);
    const msiForAll = { ...good, platforms: { ...good.platforms, "windows-x86_64": entry("33") } };
    assert.match(latestJsonProblems(msiForAll, "0.2.0", assets).join(), /windows-x86_64 points at .*\.msi, not the -setup\.exe payload/);
    // A dry run's would-be latest.json passes the same gate; a missing signature fails it.
    const bundles = ["IssueRelay.app.tar.gz", "IssueRelay_0.2.0_x64-setup.exe", "IssueRelay_0.2.0_x64_en-US.msi"].map((name) => ({ id: name, name }));
    assert.deepEqual(latestJsonProblems(wouldBeLatestJson("0.2.0", bundles, () => "c2ln"), "0.2.0", bundles), []);
    assert.match(latestJsonProblems(wouldBeLatestJson("0.2.0", bundles, () => ""), "0.2.0", bundles).join(), /has no signature/);
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
  tag: {
    options: { tag: { type: "string" }, manifest: { type: "string" } },
    run: (values) => tagProblems(required(values, "tag"), workspaceVersion(readFileSync(values.manifest ?? path.join(root, "Cargo.toml"), "utf8"))),
  },
  "on-main": {
    options: { commit: { type: "string" }, ref: { type: "string" } },
    run: (values) => onMainProblems(required(values, "commit"), values.ref ?? "origin/main"),
  },
  assets: {
    options: { version: { type: "string" }, updater: { type: "string" }, mode: { type: "string" }, names: { type: "string" } },
    run: (values) => assetProblems(lines(required(values, "names")), required(values, "version"), choice(values, "updater", ["true", "false"]) === "true", choice(values, "mode", ["release", "dry-run"])),
  },
  "latest-json": {
    options: { version: { type: "string" }, latest: { type: "string" }, assets: { type: "string" } },
    run: (values) => {
      const assets = lines(required(values, "assets")).map((line) => {
        const [id, name = ""] = line.split("\t");
        return { id, name };
      });
      return latestJsonProblems(JSON.parse(readFileSync(required(values, "latest"), "utf8")), required(values, "version"), assets);
    },
  },
  "latest-json-dry-run": {
    options: { version: { type: "string" }, dir: { type: "string" } },
    run: (values) => {
      const files = readdirSync(required(values, "dir"), { recursive: true, withFileTypes: true })
        .filter((entry) => entry.isFile())
        .map((entry) => path.join(entry.parentPath, entry.name));
      const assets = files.map((file) => ({ id: path.basename(file), name: path.basename(file) }));
      const signatureOf = (name) => {
        const sig = files.find((file) => path.basename(file) === `${name}.sig`);
        return sig === undefined ? "" : readFileSync(sig, "utf8").trim();
      };
      const version = required(values, "version");
      return latestJsonProblems(wouldBeLatestJson(version, assets, signatureOf), version, assets);
    },
  },
};

function required(values, name) {
  if (values[name] === undefined) throw new Error(`--${name} is required`);
  return values[name];
}

function choice(values, name, allowed) {
  const value = required(values, name);
  if (!allowed.includes(value)) throw new Error(`--${name} is one of ${allowed.join(", ")}`);
  return value;
}

/** The non-empty lines of a file. */
function lines(file) {
  return readFileSync(file, "utf8")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
}

function main(argv) {
  const [name, ...rest] = argv;
  if (name === "self-test" && rest.length === 0) return selfTest();
  if (name === "workspace-version" && rest.length === 0) {
    console.log(workspaceVersion(readFileSync(path.join(root, "Cargo.toml"), "utf8")));
    return 0;
  }
  const gate = GATES[name];
  if (gate === undefined) throw new Error(`usage: node scripts/release-checks.mjs <${[...Object.keys(GATES), "workspace-version", "self-test"].join("|")}> [options]`);
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

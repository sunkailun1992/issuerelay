#!/usr/bin/env node
// Checks Tauri updater signatures (minisign) against the public key committed in
// app/src-tauri/tauri.conf.json, and that each signature's trusted comment names the release
// version. `tauri build` only warns when the signing key does not match the public key, so the
// release workflow runs this on every update payload; a mismatch fails the release.
//
// Usage: node scripts/verify-update-signature.mjs --version <X.Y.Z> --artifacts '<JSON array>' [--pubkey <base64>]
//   --artifacts  tauri-action's `artifactPaths` output; every `<file>.sig` in it is checked
//                against `<file>`, and at least one must be there
//   --pubkey     another public key instead of the committed one (the local self-test)
// Exit codes: 0 = every signature verified, 1 = one did not, 2 = bad arguments or unreadable files.
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// DER header of an Ed25519 SubjectPublicKeyInfo; the 32 key bytes follow it.
const ED25519_SPKI = Buffer.from("302a300506032b6570032100", "hex");

/** The text lines of a base64-encoded minisign box, the way Tauri stores keys and signatures. */
function boxLines(base64Text) {
  return Buffer.from(base64Text.trim(), "base64").toString("utf8").split("\n").map((line) => line.replace(/\r$/, ""));
}

function publicKey(base64Key) {
  const [comment, encoded] = boxLines(base64Key);
  if (!comment?.startsWith("untrusted comment:") || !encoded) throw new Error("the pubkey is not a minisign public key");
  const raw = Buffer.from(encoded, "base64");
  if (raw.length !== 42 || raw.subarray(0, 2).toString("latin1") !== "Ed") throw new Error("unexpected minisign public key format");
  return {
    keyId: raw.subarray(2, 10),
    key: createPublicKey({ key: Buffer.concat([ED25519_SPKI, raw.subarray(10)]), format: "der", type: "spki" }),
  };
}

/** Why `payload` does not verify against `pub`, or null when it does. */
function problem(pub, payload, version) {
  const [, sigLine, trustedLine, globalLine] = boxLines(readFileSync(`${payload}.sig`, "utf8"));
  const sig = Buffer.from(sigLine ?? "", "base64");
  if (sig.length !== 74) return "the .sig file is not a minisign signature";
  if (!sig.subarray(2, 10).equals(pub.keyId)) {
    return "it was signed with another key: the TAURI_SIGNING_PRIVATE_KEY secret does not match the committed pubkey";
  }
  const data = readFileSync(payload);
  // "ED" signs the BLAKE2b-512 hash of the file (what Tauri writes); "Ed" signs the file itself.
  const message = sig.subarray(0, 2).toString("latin1") === "ED" ? createHash("blake2b512").update(data).digest() : data;
  if (!verify(null, message, pub.key, sig.subarray(10))) return "the signature does not match the file";
  if (!trustedLine?.startsWith("trusted comment: ")) return "the signature has no trusted comment";
  const trusted = trustedLine.slice("trusted comment: ".length);
  const globalSig = Buffer.from(globalLine ?? "", "base64");
  if (!verify(null, Buffer.concat([sig.subarray(10), Buffer.from(trusted, "utf8")]), pub.key, globalSig)) {
    return "the trusted comment was altered";
  }
  if (!trusted.split("\t").includes(`version:${version}`)) {
    return `the signature does not name version ${version} (trusted comment: ${trusted})`;
  }
  return null;
}

function main() {
  const { values } = parseArgs({
    options: { version: { type: "string" }, artifacts: { type: "string" }, pubkey: { type: "string" } },
  });
  if (!values.version || !values.artifacts) {
    throw new Error("usage: node scripts/verify-update-signature.mjs --version <X.Y.Z> --artifacts '<JSON array>' [--pubkey <base64>]");
  }
  const signatures = JSON.parse(values.artifacts).filter((file) => file.endsWith(".sig"));
  if (signatures.length === 0) throw new Error("no update signature among the artifacts");
  const config = JSON.parse(readFileSync(path.join(root, "app/src-tauri/tauri.conf.json"), "utf8"));
  const pub = publicKey(values.pubkey ?? config.plugins?.updater?.pubkey ?? "");
  let failed = false;
  for (const signature of signatures) {
    const payload = signature.slice(0, -".sig".length);
    const why = problem(pub, payload, values.version);
    console.log(why === null ? `verified ${path.basename(payload)}` : `FAILED ${path.basename(payload)}: ${why}`);
    failed ||= why !== null;
  }
  return failed ? 1 : 0;
}

try {
  process.exitCode = main();
} catch (error) {
  console.error(`verify-update-signature: ${error.message}`);
  process.exitCode = 2;
}

#!/usr/bin/env bash
# Self-test of scripts/check-secrets.sh. Copies the script into a throwaway git repository that
# holds only made-up lines and checks, with the real script, that
#   - every kind of value it guards against is reported, as file:line and never as the matched
#     text,
#   - near misses (a public key, a short key-like line, words that merely contain a prefix, other
#     address ranges, system paths) are not,
#   - a repository without hits exits 0, hits exit 1 only with --strict, and a bad argument or a
#     scan that cannot run exits 2.
# The made-up values are put together here from pieces, so this file matches none of the patterns
# itself, and none of them is a real key, token or address. CI runs this before the real scan, so
# an edit to a pattern cannot quietly stop it from catching what it was added for. Edit a pattern
# in check-secrets.sh and its samples here together.
#
# Usage: bash scripts/check-secrets-self-test.sh
# Exit codes: 0 = every check passed; 1 = a check failed (each is named on stderr);
#             2 = the self-test itself could not run
set -euo pipefail
trap 'echo "check-secrets-self-test: could not run (a command exited with $?)" >&2; exit 2' ERR
cd "$(dirname "$0")/.."

work=$(mktemp -d)
bare=$(mktemp -d)
trap 'rm -rf "$work" "$bare"' EXIT

failures=0
fail() {
  echo "check-secrets-self-test: FAIL: $1" >&2
  failures=$((failures + 1))
}

# rep <character> <count>: that many copies of the character.
rep() { printf '%*s' "$2" '' | tr ' ' "$1"; }
# put <hit|miss> <name> <content>: one file with one line; `hit` files must be reported at line 1,
# `miss` files must not be reported at all.
put() { printf '%s\n' "$3" >"$work/$1-$2.txt"; }

mkdir "$work/scripts"
cp scripts/check-secrets.sh "$work/scripts/check-secrets.sh"
git -C "$work" init -q

# The pieces the samples are made of: written in two halves wherever the whole would match.
ten=10
lan=192.168
mid=172
home=/ho
users=/Us

# ---- what must be reported ---------------------------------------------------------------------
put hit private-key "-----BEGIN RS""A PRIVATE KEY-----"
put hit private-key-ec "-----BEGIN E""C PRIVATE KEY-----"
put hit private-key-openssh "-----BEGIN OPENS""SH PRIVATE KEY-----"
put hit linear-api-key "lin_api_$(rep a 24)"
put hit linear-oauth-token "lin_oauth_$(rep b 24)"
put hit anthropic-key "key = sk-ant-$(rep c 30)"
put hit openai-key "OPENAI=sk-$(rep e 30)"
put hit key-after-an-escaped-newline '{"text":"x\nsk-'"$(rep d 24)"'"}'
put hit github-token "ghp_$(rep f 36)"
put hit github-fine-grained-token "github_pat_$(rep g 40)"
put hit aws-access-key-id "AKIA$(rep A 16)"
# A Tauri updater signing key, in each form it can turn up in (docs/release/updater-key.md):
# the minisign secret key file as text, that file base64-encoded as `tauri signer generate`
# writes it, and its second line alone, base64 of the key itself, which starts the same for every
# key and is 212 characters long.
put hit updater-key-text "untrusted comment: rsign encrypted secret k""ey"
put hit updater-key-file-base64 "dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V""5$(rep Q 40)"
put hit updater-key-line "RWRT""Y0Iy$(rep A 150)"
put hit updater-key-line-in-quotes "\"RWRT""Y0Iy$(rep B 150)\""
put hit updater-key-line-after-a-colon "key: RWRT""Y0Iy$(rep C 150)"
put hit private-address-ten "host: $ten.20.30.40"
put hit private-address-192 "$lan.1.15"
put hit private-address-172-low "$mid.16.0.9"
put hit private-address-172-high "$mid.31.255.254"
put hit absolute-path-macos "$users""ers/someone/project/file"
put hit absolute-path-linux "${home}me/someone/project"
put hit absolute-path-volume "/Vol""umes/Disk/folder/"
put hit absolute-path-windows "C:\\Us""ers\\someone"
put hit invisible-zero-width-space "a$(printf '\xe2\x80\x8b')b"
put hit direction-override "a$(printf '\xe2\x80\xae')b"
put hit control-character "a$(printf '\x07')b"

# ---- what must stay quiet ----------------------------------------------------------------------
# The updater's public key (a minisign public key line is 56 characters and starts with the
# algorithm and the key number, not with a secret key's header) and a public key file's base64.
put miss updater-public-key-line "RWQ$(rep A 53)"
put miss updater-public-key-file-base64 "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6$(rep A 80)"
put miss updater-key-header-alone "RWRT""Y0Iy"
put miss updater-key-line-too-short "RWRT""Y0Iy$(rep A 60)"
put miss updater-key-line-inside-a-word "xRWRT""Y0Iy$(rep A 150)"
put miss task-is-not-a-key "task-$(rep h 30)"
put miss other-address-ranges "$mid.32.0.1 1$ten.1.2.3 1.$ten.2.3 192.169.0.1 8.8.8.8"
put miss system-paths "/usr/local/bin /var/tmp/x ~/Library/Logs"
put miss plain-text "Nothing here is secret."

git -C "$work" add -A

# ---- the real script on those files ------------------------------------------------------------
status=0
out=$(cd "$work" && bash scripts/check-secrets.sh --strict 2>&1) || status=$?
[[ $status -eq 1 ]] || fail "--strict with hits exits 1 (got $status)"

for file in "$work"/hit-*.txt; do
  name=$(basename "$file")
  grep -q "^check-secrets: $name:1" <<<"$out" || fail "$name is not reported"
  # The report names the place, never the text found there.
  if grep -qF -- "$(cat "$file")" <<<"$out"; then fail "$name: the report prints the matched text"; fi
done
for file in "$work"/miss-*.txt; do
  name=$(basename "$file")
  if grep -q "$name" <<<"$out"; then fail "$name is reported though it is no secret"; fi
done

# Without --strict hits are only reported.
status=0
(cd "$work" && bash scripts/check-secrets.sh >/dev/null 2>&1) || status=$?
[[ $status -eq 0 ]] || fail "hits without --strict exit 0 (got $status)"

# No hits: exit 0 and no output.
(cd "$work" && git rm -q -f -- hit-*.txt)
status=0
out=$(cd "$work" && bash scripts/check-secrets.sh --strict 2>&1) || status=$?
[[ $status -eq 0 && -z $out ]] || fail "a clean repository exits 0 without output (got $status: $out)"

# A bad argument is refused, and a scan that cannot run never reads as a clean one.
status=0
(cd "$work" && bash scripts/check-secrets.sh --bogus >/dev/null 2>&1) || status=$?
[[ $status -eq 2 ]] || fail "a bad argument exits 2 (got $status)"
mkdir "$bare/scripts"
cp scripts/check-secrets.sh "$bare/scripts/check-secrets.sh"
status=0
(cd "$bare" && bash scripts/check-secrets.sh --strict >/dev/null 2>&1) || status=$?
[[ $status -eq 2 ]] || fail "outside a git checkout exits 2 (got $status)"

if [[ $failures -gt 0 ]]; then
  echo "check-secrets-self-test: $failures check(s) failed" >&2
  exit 1
fi
echo "check-secrets-self-test: ok"

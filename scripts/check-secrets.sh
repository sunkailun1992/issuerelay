#!/usr/bin/env bash
# Scan tracked files for values that must never be committed: private keys,
# Linear / Anthropic / OpenAI / GitHub / AWS tokens, private network addresses
# and machine-specific absolute paths; and for raw invisible, direction-changing
# and control characters (Trojan Source: code that reads differently from what
# runs; SECURITY_CODING_SPEC.md section 7), which needs a git built with PCRE2
# (`git grep -P`). Prints file:line for each hit, never the matched text, so CI
# logs cannot leak a secret. New files are scanned only after `git add`.
# `git grep -I` skips files Git treats as binary (a NUL byte near the start, or
# a `binary` / `-diff` attribute in .gitattributes), so samples and fixtures
# must stay plain text to be scanned.
#
# Usage: bash scripts/check-secrets.sh [--strict]
# Exit codes: 0 = no hits, or hits without --strict
#             1 = hits with --strict
#             2 = bad arguments, or the scan itself failed: git grep exited
#                 above 1 or reported any error (outside a git checkout, an
#                 invalid pattern, an unreadable file), or any other command
#                 failed; it never fails open
set -euo pipefail
# Any failing command ends the script through `set -e`; the trap makes that
# exit code 2, so a broken scan never reads as "hits found" (1).
trap 'echo "check-secrets: scan failed (a command exited with $?)" >&2; exit 2' ERR
cd "$(dirname "$0")/.."

usage() { echo "usage: bash scripts/check-secrets.sh [--strict]"; }

STRICT=0
case "$#:${1-}" in
  0:) ;;
  1:--strict) STRICT=1 ;;
  1:-h | 1:--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

PATTERNS=(
  'BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY'
  'lin_(api|oauth)_[A-Za-z0-9]{16,}'
  # Anthropic (sk-ant-...) and OpenAI (sk-..., sk-proj-...) keys. The left
  # boundary keeps words such as "task-..." from matching; an escaped \n, \r
  # or \t also counts as a boundary so keys inside JSON strings are caught.
  '(^|[^A-Za-z0-9_-]|\\[nrt])sk-[A-Za-z0-9_-]{20,}'
  'gh[pousr]_[A-Za-z0-9]{30,}'
  'github_pat_[A-Za-z0-9_]{30,}'
  'AKIA[0-9A-Z]{16}'
  '(^|[^0-9.])10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}'
  '(^|[^0-9.])192\.168\.[0-9]{1,3}\.[0-9]{1,3}'
  '(^|[^0-9.])172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3}'
  '/Users/[A-Za-z0-9._-]+/'
  '/home/[A-Za-z0-9._-]+/'
  '/Volumes/[^ /]+/'
  '[A-Za-z]:\\Users\\[A-Za-z0-9._-]+'
)
EXCLUDES=(':!scripts/check-secrets.sh' ':!Cargo.lock' ':!LICENSE')
# Control characters other than tab, newline and carriage return, and the
# characters text::is_invisible removes (zero-width, direction marks and
# overrides, invisible operators, the byte order mark, the soft hyphen, Hangul
# fillers, tag characters). Written as escapes, so this file has none itself and
# is scanned like every other file.
INVISIBLE='(*UTF)[\x{01}-\x{08}\x{0B}\x{0C}\x{0E}-\x{1F}\x{7F}-\x{9F}\x{AD}\x{61C}\x{115F}\x{1160}\x{200B}-\x{200F}\x{2028}-\x{202E}\x{2060}-\x{206F}\x{3164}\x{FEFF}\x{FFA0}\x{E0000}-\x{E007F}]'

err=$(mktemp)
trap 'rm -f "$err"' EXIT

hits=0
for p in "${PATTERNS[@]}"; do
  code=0
  out=$(git grep -n -I -E -e "$p" -- . "${EXCLUDES[@]}" 2>"$err") || code=$?
  # git grep exits 1 when nothing matched; anything above 1, or any message
  # on stderr (it reports unreadable files there but still exits 0 or 1),
  # means the scan did not cover everything.
  if [[ $code -gt 1 || -s "$err" ]]; then
    echo "check-secrets: scan failed (git grep exit $code) for pattern: $p" >&2
    sed 's/^/check-secrets: /' "$err" >&2
    exit 2
  fi
  [[ $code -eq 1 ]] && continue
  while IFS= read -r loc; do
    echo "check-secrets: $loc"
    hits=$((hits + 1))
  done < <(printf '%s\n' "$out" | cut -d: -f1,2)
done

code=0
out=$(git grep -n -I -P -e "$INVISIBLE" -- . 2>"$err") || code=$?
if [[ $code -gt 1 || -s "$err" ]]; then
  echo "check-secrets: scan failed (git grep -P exit $code) for invisible characters" >&2
  sed 's/^/check-secrets: /' "$err" >&2
  exit 2
fi
if [[ $code -eq 0 ]]; then
  while IFS= read -r loc; do
    echo "check-secrets: $loc (a raw invisible or control character: write it as an escape)"
    hits=$((hits + 1))
  done < <(printf '%s\n' "$out" | cut -d: -f1,2)
fi

if [[ $hits -gt 0 ]]; then
  echo "check-secrets: $hits hit(s)"
  [[ $STRICT -eq 1 ]] && exit 1
fi
exit 0

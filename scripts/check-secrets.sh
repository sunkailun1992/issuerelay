#!/usr/bin/env bash
# Scan tracked files for values that must never be committed: private keys,
# Linear / Anthropic / OpenAI / GitHub / AWS tokens, private network addresses
# and machine-specific absolute paths. Prints file:line for each hit, never the
# matched text, so CI logs cannot leak a secret. New files are scanned only
# after `git add`.
#
# Usage: bash scripts/check-secrets.sh [--strict]
# Exit codes: 0 = no hits, or hits without --strict
#             1 = hits with --strict
#             2 = bad arguments, or the scan itself failed (for example outside
#                 a git checkout or with an invalid pattern); it never fails open
set -euo pipefail
cd "$(dirname "$0")/.."

usage() { echo "usage: bash scripts/check-secrets.sh [--strict]"; }

STRICT=0
if [[ $# -gt 1 ]]; then
  usage >&2
  exit 2
fi
case "${1:-}" in
  "") ;;
  --strict) STRICT=1 ;;
  -h | --help)
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
  # Anthropic (sk-ant-...) and OpenAI (sk-..., sk-proj-...) keys; the left
  # boundary keeps words such as "task-..." from matching.
  '(^|[^A-Za-z0-9_-])sk-[A-Za-z0-9_-]{20,}'
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

hits=0
for p in "${PATTERNS[@]}"; do
  code=0
  out=$(git grep -n -I -E -e "$p" -- . "${EXCLUDES[@]}") || code=$?
  if [[ $code -eq 1 ]]; then
    continue # git grep exits 1 when nothing matched
  elif [[ $code -ne 0 ]]; then
    echo "check-secrets: scan failed (git grep exit $code) for pattern: $p" >&2
    exit 2
  fi
  while IFS= read -r loc; do
    echo "check-secrets: $loc"
    hits=$((hits + 1))
  done < <(printf '%s\n' "$out" | cut -d: -f1,2)
done

if [[ $hits -gt 0 ]]; then
  echo "check-secrets: $hits hit(s)"
  [[ $STRICT -eq 1 ]] && exit 1
fi
exit 0

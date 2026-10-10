#!/usr/bin/env bash
# Scan tracked files for values that must never be committed: private keys,
# Linear / Anthropic / OpenAI / GitHub / AWS tokens, private network addresses
# and machine-specific absolute paths. Prints file:line for each hit. Exits 0
# unless --strict is given and something matched. New files are scanned only
# after `git add`.
set -euo pipefail
cd "$(dirname "$0")/.."
STRICT=0
[[ "${1:-}" == "--strict" ]] && STRICT=1

PATTERNS=(
  'BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY'
  'lin_(api|oauth)_[A-Za-z0-9]{16,}'
  'sk-ant-[A-Za-z0-9_-]{16,}'
  'sk-[A-Za-z0-9]{32,}'
  'gh[pousr]_[A-Za-z0-9]{30,}'
  'AKIA[0-9A-Z]{16}'
  '(^|[^0-9.])10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}'
  '(^|[^0-9.])192\.168\.[0-9]{1,3}\.[0-9]{1,3}'
  '(^|[^0-9.])172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3}'
  '/Users/[A-Za-z0-9._-]+/'
  '/Volumes/[^ /]+/'
  '[A-Za-z]:\\Users\\[A-Za-z0-9._-]+'
)
EXCLUDES=(':!scripts/check-secrets.sh' ':!Cargo.lock' ':!LICENSE')

hits=0
for p in "${PATTERNS[@]}"; do
  while IFS= read -r loc; do
    [[ -z "$loc" ]] && continue
    echo "check-secrets: $loc"
    hits=$((hits + 1))
  done < <(git grep -n -I -E -e "$p" -- . "${EXCLUDES[@]}" | cut -d: -f1,2 || true)
done

if [[ $hits -gt 0 ]]; then
  echo "check-secrets: $hits hit(s)"
  [[ $STRICT -eq 1 ]] && exit 1
fi
exit 0

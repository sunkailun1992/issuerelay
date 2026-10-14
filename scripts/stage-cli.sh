#!/usr/bin/env bash
# Builds the `issuerelay` CLI in release mode and stages it where Tauri's externalBin looks for
# it (app/src-tauri/tauri.bundle.conf.json): app/src-tauri/binaries/issuerelay-<target triple>,
# with .exe for Windows targets. Tauri strips the triple when it bundles the file next to the
# app's executable.
#
#   (no argument)       the host's triple, for `tauri build` without --target
#   --target <triple>   one triple
#   --universal         aarch64-apple-darwin and x86_64-apple-darwin, plus the lipo-merged
#                       issuerelay-universal-apple-darwin that `tauri build --target
#                       universal-apple-darwin` copies as it is: the build script needs all three
#
# Usage: bash scripts/stage-cli.sh [--target <triple> | --universal]
# Exit codes: 0 = staged, 2 = bad arguments, anything else = a failed build or copy.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() { echo "usage: bash scripts/stage-cli.sh [--target <triple> | --universal]"; }

out=app/src-tauri/binaries
mode=host
target=""
case "$#:${1-}" in
  0:) ;;
  1:--universal) mode=universal ;;
  2:--target)
    mode=target
    target=$2
    ;;
  1:-h | 1:--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

# Builds the CLI for one triple and copies it into place.
stage() {
  local triple=$1 ext=""
  [[ $triple == *-windows-* ]] && ext=.exe
  cargo build -p issuerelay-cli --release --locked --target "$triple"
  mkdir -p "$out"
  cp "target/$triple/release/issuerelay$ext" "$out/issuerelay-$triple$ext"
  echo "staged $out/issuerelay-$triple$ext"
}

case $mode in
  host) stage "$(rustc --print host-tuple)" ;;
  target) stage "$target" ;;
  universal)
    stage aarch64-apple-darwin
    stage x86_64-apple-darwin
    lipo -create -output "$out/issuerelay-universal-apple-darwin" \
      "$out/issuerelay-aarch64-apple-darwin" "$out/issuerelay-x86_64-apple-darwin"
    archs=$(lipo -archs "$out/issuerelay-universal-apple-darwin")
    [[ $archs == *x86_64* && $archs == *arm64* ]] || {
      echo "stage-cli: the universal CLI holds only: $archs" >&2
      exit 1
    }
    echo "staged $out/issuerelay-universal-apple-darwin ($archs)"
    ;;
esac

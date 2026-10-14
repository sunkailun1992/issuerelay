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
#                       universal-apple-darwin` copies as it is: the build script needs all three.
#                       Needs lipo and both Rust targets installed; checked before anything builds.
#
# Usage: bash scripts/stage-cli.sh [--target <triple> | --universal]
# Exit codes: 0 = staged, 2 = bad arguments, anything else = a missing tool or Rust target for
# --universal, or a failed build or copy.
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
  host)
    # Its own statement: inside `stage "$(...)"` a failing rustc would not stop the script.
    triple=$(rustc --print host-tuple)
    [[ -n $triple ]] || {
      echo "stage-cli: rustc printed no host triple" >&2
      exit 1
    }
    stage "$triple"
    ;;
  target) stage "$target" ;;
  universal)
    # Before anything is compiled: both builds take minutes, and these would fail only after them.
    command -v lipo > /dev/null || {
      echo "stage-cli: --universal needs lipo, which comes with macOS (the Xcode command line tools)" >&2
      exit 1
    }
    command -v rustup > /dev/null || {
      echo "stage-cli: --universal needs rustup, to see which Rust targets are installed" >&2
      exit 1
    }
    installed=$(rustup target list --installed)
    for triple in aarch64-apple-darwin x86_64-apple-darwin; do
      grep -qx "$triple" <<< "$installed" || {
        echo "stage-cli: the Rust target $triple is not installed: run 'rustup target add $triple'" >&2
        exit 1
      }
    done
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

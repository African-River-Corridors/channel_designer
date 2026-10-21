#!/usr/bin/env bash
# Build the design session for the browser (spec 0011): owd-web for wasm32, then the wasm-bindgen glue
# into app/wasm-pkg/. The CLI version must equal the `wasm-bindgen` crate version pinned in
# crates/owd-web/Cargo.toml; install it with:
#   cargo install wasm-bindgen-cli --version <pinned> --locked
# owd-format bundles SQLite (the GeoPackage reader), so the build needs a clang that can target wasm32
# (Ubuntu's clang can; Apple's cannot: set CC_wasm32_unknown_unknown to a Homebrew llvm clang).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="app/wasm-pkg"
pinned="$(sed -n 's/^wasm-bindgen = "=\([0-9.]*\)".*/\1/p' crates/owd-web/Cargo.toml)"
[ -n "$pinned" ] || { echo "build_web: cannot read the pinned wasm-bindgen version" >&2; exit 1; }
command -v wasm-bindgen >/dev/null 2>&1 \
  || { echo "build_web: wasm-bindgen CLI not found (cargo install wasm-bindgen-cli --version $pinned --locked)" >&2; exit 1; }
cli="$(wasm-bindgen --version | awk '{print $2}')"
[ "$cli" = "$pinned" ] \
  || { echo "build_web: wasm-bindgen CLI is $cli but the crate is pinned to $pinned" >&2; exit 1; }

cargo build --target wasm32-unknown-unknown --release -p owd-web
rm -rf "$OUT"
wasm-bindgen --target web --out-dir "$OUT" "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/owd_web.wasm"
echo "build_web: wrote $OUT (wasm-bindgen $cli)"

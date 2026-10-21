#!/usr/bin/env bash
# Run the web smoke test: build the web app in test mode (it exposes a read-only hook the test uses),
# then drive headless Chrome against it. Needs the WebAssembly package (bash tools/build_web.sh), Chrome,
# and the CLI built in release mode (cargo build --release -p owd-cli).
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f wasm-pkg/owd_web_bg.wasm ] || { echo "e2e-web: no app/wasm-pkg (run bash tools/build_web.sh)" >&2; exit 1; }
target="${CARGO_TARGET_DIR:-$(cd .. && pwd)/target}"
export OWD_CLI="${OWD_CLI:-$target/release/owd}"
[ -x "$OWD_CLI" ] || { echo "e2e-web: no CLI at $OWD_CLI (cargo build --release -p owd-cli)" >&2; exit 1; }
npx vite build --config vite.web.config.ts --mode test --outDir dist-web-test
node --test --test-timeout=300000 e2e/web-smoke.test.mjs

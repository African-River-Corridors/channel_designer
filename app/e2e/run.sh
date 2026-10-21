#!/usr/bin/env bash
# Run the smoke tests against the built debug app. Needs `tauri-driver`, WebKitWebDriver and a
# display (CI wraps this in xvfb-run). Build the app first:  npx tauri build --debug --no-bundle
set -euo pipefail
cd "$(dirname "$0")/.."
command -v tauri-driver >/dev/null || { echo "e2e: tauri-driver is not on PATH (cargo install tauri-driver --locked)" >&2; exit 1; }
# A debug build answers its native dialogs from files in this folder (see `test_answer` in src-tauri/src/lib.rs).
export CD_TEST_DIALOG_DIR="$(mktemp -d)"
# Every basemap tile request fails (nothing listens on port 9), so the map is offline and the hillshade shows.
export CD_TEST_TILE_URL="http://127.0.0.1:9/{z}/{x}/{y}.jpg"
export CD_APP_BINARY="${CD_APP_BINARY:-../../target/debug/owd-app}"
[ -x "$(cd e2e && realpath "$CD_APP_BINARY")" ] || { echo "e2e: no built app at $CD_APP_BINARY" >&2; exit 1; }
# Software rendering under a virtual display (a missing GPU can stall WebKitGTK for a long time).
export LANG=en_US.UTF-8 LANGUAGE=en_US LC_ALL=en_US.UTF-8
export WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1 GDK_BACKEND=x11
tauri-driver --port 4444 &
driver=$!
trap 'kill "$driver" 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do
  (echo > /dev/tcp/127.0.0.1/4444) 2>/dev/null && break
  sleep 1
done
node --test e2e/smoke.test.mjs

#!/usr/bin/env bash
# Screenshot the mock session (src/mock.ts) in headless Chrome, light and dark.
# Needs `npm run dev` running on port 1420. Usage: scripts/shot.sh <name> [outdir]
# HEIGHT=1200 makes the window taller. Writes <outdir>/<name>-light.png and <name>-dark.png (default outdir: /tmp/owd-shots).
set -euo pipefail
name=${1:?usage: shot.sh <name> [outdir]}
out=${2:-/tmp/owd-shots}
chrome=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
mkdir -p "$out"
for theme in light dark; do
  "$chrome" --headless=new --hide-scrollbars --window-size=1440,${HEIGHT:-900} --virtual-time-budget=20000 \
    --use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader --ignore-gpu-blocklist \
    --screenshot="$out/$name-$theme.png" "http://localhost:1420/mock.html?theme=$theme" >/dev/null 2>&1
done
echo "$out/$name-light.png $out/$name-dark.png"

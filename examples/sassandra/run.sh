#!/usr/bin/env bash
# The Sassandra worked example (spec 0012, journey 1): what barge fits the Sassandra, Côte d'Ivoire, from
# the sea at Sassandra town up to the Soubré dam? From a river name and two points, with nothing of
# our own: the route, banks, imagery, DEM and water level are fetched at run time from open data (a few
# hundred MB, kept in the study folder). See docs/examples/sassandra.md for the result and its limits.
#
#   bash examples/sassandra/run.sh            # writes ./sassandra-study/
#   OWD=/path/to/owd bash examples/sassandra/run.sh
#
# Needs the network. Takes about 40 minutes, most of it the reach step (Overpass and the Sentinel-2 scenes).
set -euo pipefail
OWD="${OWD:-owd}"
OUT="${1:-sassandra-study}"
mkdir -p "$OUT"
cd "$OUT"
DOC=sassandra.owd

# 1. The reach: the sea at Sassandra town to just below the Soubré dam, on the OSM waterways. Bank lines
#    seeded from the water in dry-season Sentinel-2 scenes (spec 0013), with the JRC water mask at 30 % (the
#    default 75 % sees almost no water on this cloudy reach), checked, repaired and moved onto the scenes'
#    edges, tile by tile.
#    Its bank checks exit 1 with a warning, and still write the study. On this river the banks still cross
#    (`banks_cross`, a limit the doc states), so go on past that warning only.
status=0
"$OWD" reach "$DOC" --river Sassandra --start=-6.0780,4.9605 --end=-6.6550,5.8020 \
  --banks --mask-threshold 30 --edge-fit --json > reach.json || status=$?
if [ "$status" -ne 0 ]; then
  others=$(grep -o '"check": *"[a-z_]*"' reach.json | grep -v '"banks_cross"' || true)
  if [ "$status" -ne 1 ] || [ -n "$others" ] || ! grep -q '"banks_cross"' reach.json; then
    echo "owd reach failed (exit $status): see $OUT/reach.json" >&2
    exit "$status"
  fi
  {
    echo "################################################################################"
    echo "WARNING: owd reach exited 1. Its one warning is banks_cross: the left and right"
    echo "banks cross (24 places on the 2026-10-23 run, km 24 to 109). The study is written,"
    echo "so the script goes on. The width stretches near the crossings are not to be"
    echo "trusted. See docs/examples/sassandra.md, \"Limits\"."
    echo "################################################################################"
  } >&2
fi

# 2. The DEM (Copernicus GLO-30) and the default low-water level (SWORD and SWOT, the DEM where they give none).
"$OWD" fetch-dem "$DOC" --json > dem.json
"$OWD" water-level "$DOC" --json > water-level.json

# 3. The plan-only answer: width and radius, no depth, read on the reference line (the default plan line,
#    ADR 0014), which this step makes from the banks and stores in the study.
"$OWD" fits "$DOC" --json > fits-plan-only.json
"$OWD" fits "$DOC" --show > fits-plan-only.txt

# 4. The depth people who use the river give: 3 m at low water in the dry season, assumed, not measured.
"$OWD" bed "$DOC" --depth 3 --season dry --json > bed.json

# 5. The full answer: width, radius and depth, with the spec 0007 verdict per class, all read along the
#    reference line.
"$OWD" fits "$DOC" --json > fits-full.json
"$OWD" fits "$DOC" --show > fits-full.txt

# 6. The same full answer with the radius read on the river centreline instead, to compare: the centreline
#    drawn from the banks wiggles at the scale of its stations, and every wiggle reads as a tight bend.
cp "$DOC" sassandra-centreline.owd
"$OWD" fits sassandra-centreline.owd --plan-line centreline --json > fits-full-centreline.json
"$OWD" fits sassandra-centreline.owd --show > fits-full-centreline.txt

# 7. The reports (spec 0014): all four, as PDF, HTML and Word. The study stops at the fits answer, so it has no
#    design line option: the quantity schedule and the section pack say so.
"$OWD" report "$DOC" --kind all --format all -o reports --json > reports.json

echo "wrote $OUT/: $DOC, the answers (fits-plan-only.*, fits-full.*) and the reports (reports/)"

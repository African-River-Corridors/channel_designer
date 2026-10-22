#!/usr/bin/env bash
# The Sassandra worked example (spec 0012, journey 1): what barge fits the Sassandra, Côte d'Ivoire, from
# the sea at Sassandra town up to the Soubré dam? From a river name and two points, with nothing of
# our own: the route, banks, imagery, DEM and water level are fetched at run time from open data (a few
# hundred MB, kept in the study folder). See docs/examples/sassandra.md for the result and its limits.
#
#   bash examples/sassandra/run.sh            # writes ./sassandra-study/
#   OWD=/path/to/owd bash examples/sassandra/run.sh
#
# Needs the network. Takes about 20 minutes, most of it the route (Overpass) and the edge fit.
set -euo pipefail
OWD="${OWD:-owd}"
OUT="${1:-sassandra-study}"
mkdir -p "$OUT"
cd "$OUT"
DOC=sassandra.owd

# 1. The reach: the sea at Sassandra town to just below the Soubré dam, on the OSM waterways. Bank lines
#    from the JRC water mask at 30 % (the default 75 % sees almost no water on this cloudy reach), moved
#    onto dry-season Sentinel-2 scenes, tile by tile.
"$OWD" reach "$DOC" --river Sassandra --start=-6.0780,4.9605 --end=-6.6550,5.8020 \
  --banks --mask-threshold 30 --edge-fit --json > reach.json

# 2. The DEM (Copernicus GLO-30) and the default low-water level (SWORD and SWOT, the DEM where they give none).
"$OWD" fetch-dem "$DOC" --json > dem.json
"$OWD" water-level "$DOC" --json > water-level.json

# 3. The plan-only answer: width and radius, no depth.
"$OWD" fits "$DOC" --json > fits-plan-only.json
"$OWD" fits "$DOC" --show > fits-plan-only.txt

# 4. The depth people who use the river give: 3 m at low water in the dry season, assumed, not measured.
"$OWD" bed "$DOC" --depth 3 --season dry --json > bed.json

# 5. The full answer: width, radius and depth, with the spec 0007 verdict per class.
"$OWD" fits "$DOC" --json > fits-full.json
"$OWD" fits "$DOC" --show > fits-full.txt

# 6. The same full answer with the river's radius read on each class's fairway fit (ADR 0007) instead of
#    the river centreline, an opt-in: the centreline drawn from the banks wiggles at the scale of its
#    stations, and every wiggle reads as a tight bend. Which is the default is still to be chosen.
cp "$DOC" sassandra-fairway.owd
"$OWD" fits sassandra-fairway.owd --plan-line fairway --json > fits-full-fairway.json
"$OWD" fits sassandra-fairway.owd --show > fits-full-fairway.txt

# 7. The reports (spec 0014): all four, as PDF, HTML and Word. The study stops at the fits answer, so it has no
#    design line option: the quantity schedule and the section pack say so.
"$OWD" report "$DOC" --kind all --format all -o reports --json > reports.json

echo "wrote $OUT/: $DOC, the answers (fits-plan-only.*, fits-full.*) and the reports (reports/)"

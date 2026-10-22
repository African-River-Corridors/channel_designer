---
status: accepted
---
# The water seed is a Sentinel-2 mask; JRC is the fallback and the cross-check

Spec 0012 seeds the bank lines from the JRC Global Surface Water occurrence and moves them onto a
Sentinel-2 scene by an edge fit that shifts a bank at most 30 m. On the Sassandra (spec 0013) the JRC
seed is a river width wrong in places: occurrence averages 1984 to 2021, so a reservoir filled late in
that period, or seasonal water, falls under any useful threshold. A refiner cannot move a bank a river
width, and widening its limit lets it jump to the wrong edge.

## Decision

- **The seed is a water mask made from the Sentinel-2 scenes themselves:** MNDWI on the 10 m grid, with
  the SCL band masking cloud, cloud shadow, cirrus and no data. Scenes are ranked per part by the clear
  share of the river corridor, and up to three fill each other's gaps.
- **JRC is the fallback** only where no scene sees a pixel clear, and every such stretch is flagged
  `jrc-fallback`.
- **JRC is the cross-check:** the study records the share of stations where the two widths agree.
- **`--seed jrc` keeps the old path** for a reach with no clear scene at all, or for comparison.
- **The edge fit stays a refiner** with its spec 0012 limits. It now starts on the water.

## Considered

- **Keep JRC and widen the edge fit's shift limit.** Rejected: with a ±200 m search the fit finds the
  first strong edge, which in a braided or reservoir reach is often the wrong one. The seed must be on
  the water.
- **JRC with a lower threshold.** Rejected: 30 % already gives a 17 m median on 200 m of water at
  Sassandra. A lower threshold adds the floodplain everywhere else.
- **JRC "recurrence" or the yearly history layers.** Rejected for now: they are more files without CORS
  (more downloads on the web) and still end in 2021.
- **NDWI (green and NIR) instead of MNDWI.** Kept as a later option. MNDWI (green and SWIR) separates
  turbid and algae-rich water from vegetation better, and the scene search already reads B11.
- **Sentinel-1 radar, which sees through cloud.** Out of scope: a new source, speckle, and a much larger
  read. The multi-scene composite and the JRC fallback cover cloud for now.

## Consequences

- Every bank run reads more of each scene: the blue band (for the overlay) and the SCL over the whole
  corridor to rank scenes. On the web this is a few more range reads, all from CORS-enabled COGs.
- The JRC tiles are still needed for the fallback and the cross-check, so on the web the user still
  drops them in. A reach where every scene is clear could skip them; that is a later change.
- Generated banks move for every existing study. Each change is a design-impact entry.

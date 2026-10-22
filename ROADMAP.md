# Roadmap

Planned work, roughly in order. Each item links to its issue. Comments and pull requests are welcome on any of them.

## Next

- **Public example river on an open DEM.** ([#1](https://github.com/African-River-Corridors/open-waterway-designer/issues/1)) A real river reach run end to end on a downloadable
  DEM whose licence allows commercial reuse — **Copernicus GLO-30** (not FABDEM, which is
  non-commercial). The example downloads the tiles; nothing is bundled in the repository.
- **PyPI publication.** ([#2](https://github.com/African-River-Corridors/open-waterway-designer/issues/2)) `pip install open-waterway-designer` with trusted publishing from a tagged
  release. Until then, install from git.
- **Reports.** ([#3](https://github.com/African-River-Corridors/open-waterway-designer/issues/3)) The screening report, quantity schedule, section pack and run report as PDF,
  HTML and Word, through the [Open Report Designer](https://github.com/African-River-Corridors/open-report-designer)
  (spec 0014, in progress).
- **More rulesets.** ([#4](https://github.com/African-River-Corridors/open-waterway-designer/issues/4)) Each national standard is a ruleset: Dutch RVW, German, US (USACE EM
  1110-2-1613), Brazilian, Indian IWAI. A ruleset is coefficients as data, each with its clause,
  table and page. A method that is not coefficients plugs in as a width method
  (`owd.rules.methods`), in this package or in your own.

## Later

- **Tangent joins between optimised pools.** ([#6](https://github.com/African-River-Corridors/open-waterway-designer/issues/6)) `optimise.optimise_pools` meets the design line
  within half a lattice pitch at each pool end; close that gap with a short tangent fit.
- **Split the app layer by feature.** ([#343](https://github.com/African-River-Corridors/open-waterway-designer/issues/343)) One module per feature for the desktop commands,
  the wasm exports, the web calls and the app tests, so more work on the app can run in parallel.
- **DEM despike helper.** ([#8](https://github.com/African-River-Corridors/open-waterway-designer/issues/8)) The single-cell spike repair used before volume integration.

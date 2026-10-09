# Roadmap

Planned work, roughly in order. Each item becomes a GitHub issue labelled `roadmap` when the
repository opens. Comments and pull requests are welcome on any of them.

## Next

- **Public example river on an open DEM.** A real river reach run end to end on a downloadable
  DEM whose licence allows commercial reuse — **Copernicus GLO-30** (not FABDEM, which is
  non-commercial). The example downloads the tiles; nothing is bundled in the repository.
- **PyPI publication.** `pip install channel_designer` with trusted publishing from a tagged
  release. Until then, install from git.
- **Reports via `sankofa-doc`.** Section packs and quantity schedules as documents, once the
  document tool is released.
- **More rulesets.** Each national standard is a ruleset: Dutch RVW, German, US (USACE EM
  1110-2-1613), Brazilian, Indian IWAI. A ruleset is coefficients as data, each with its clause,
  table and page. A method that is not coefficients plugs in as a width method
  (`channel_designer.rules.methods`), in this package or in your own.

## Later

- **Smooth rolling-circle exits.** Make each spliced arc leave the old line tangentially (see
  [docs/known-issues.md](docs/known-issues.md)).
- **Tangent joins between optimised pools.** `optimise.optimise_pools` meets the design line
  within half a lattice pitch at each pool end; close that gap with a short tangent fit.
- **Plan sheets.** Plan-view drawings of the alignment, footprint and sections key, alongside the
  cross-section gallery.
- **DEM despike helper.** The single-cell spike repair used before volume integration.

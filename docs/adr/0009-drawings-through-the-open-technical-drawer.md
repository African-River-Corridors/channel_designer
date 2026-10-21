---
status: accepted
---
# Drawings go through the Open Technical Drawer, a shared crate in its own repository

OWD draws in two ways today. The Python package makes a cross-section gallery on an old API of the
Drawing Format Standard's Python tool, which the standard's 1.0 release removed. The app draws its own
long and cross sections in Svelte. Neither follows the current standard, and neither can make plan
sheets, PDF or a DXF with paper space.

The sister project, open-vessel-concept-drawings (OVCD), has `ovcd-draw`: the Drawing Format Standard
in Rust, from drawing document to normalised content, then SVG, PDF and DXF (OVCD spec 0003). It
passes the standard's conformance suite. It has no dependency on OVCD's vessel code, so it is already
domain-free. Only its name and its home tie it to OVCD.

## Decision

- **The drawing crate moves to its own public repository**, `African-River-Corridors/open-technical-drawer`
  (OTD). The crate is `otd`. It keeps `ovcd-draw`'s history and library API, and moves to standard 1.1
  (images, colour print profile, map ratios). OVCD and OWD both pin it by git tag. OWD builds no second
  drawing stack.
- **OTD owns everything that is not a domain.** The standard's conformance suite, the layout review (a
  linter on normalised content), and image handling: shaded relief from a DEM, orthophotos, and fetched
  imagery tiles in a crate apart from the core, so the core keeps no file, clock or network access.
  OTD has its own coordinator; OWD and OVCD file tickets there.
- **OWD owns the domain.** It turns a channel design document into drawing documents: which sheets,
  what each shows, the label rules, and the checks that a drawn value equals a result.
- **What a sheet shows is data, not code.** A sheet style file sets layers, label rules and their
  parameters, spacings, sizes and scales. Defaults ship with OWD. The style's hash goes on the sheet.
- **One drawing path.** OWD's Python drawing module and its `[drawings]` extra go. Python calls the
  Rust path through the binding.
- **Checks in two layers.** OTD checks format and layout. OWD checks its values, and its CI compares
  `otd`'s normalised content with the standard's Python reader for OWD's own drawing documents.

## Considered options

- **Pin `ovcd-draw` from an OVCD tag, as `arc-rules` is pinned.** Rejected: a drawing release would
  wait on a vessel tool's release, and image handling and layout rules would grow inside a vessel
  repository.
- **Put the Rust crate in the standard's repository.** Not chosen: the standard keeps its own pace and
  its Python reference reader, and a separate crate pins the standard at a known commit.
- **Port the Python gallery to the standard's 1.0 drawing documents.** Rejected: it keeps two drawing
  paths, and the app cannot use Python.

## Consequences

- OWD's drawing work waits on an OTD tag. Plan sheets wait on a further OTD release with images.
- A long section with vertical exaggeration is not possible until the standard allows it (clause 7.3.3).
  The app's own long-section view stays until then.
- Imagery terms are the user's: OWD stores no imagery key and prints the source's attribution.
- `arc-rules` stays in the OVCD repository for now (spec 0007). If it later moves out, this ADR is the
  pattern.

---
status: accepted
---
# Reports through the Open Report Designer, in their own crate

OWD gives its results as CSV, GeoJSON and drawing sheets. A decision-maker needs a report: one document
that says what the river is, what fits, where it falls short, what that costs, and on what data. Issue #3
planned this on a private Python document tool. A public tool cannot depend on a private package, and the
web app cannot run Python.

## Decision

- **Reports are documents of the Open Report Designer (ORD).** ORD is a public Rust engine (Apache-2.0):
  one typed JSON document in, PDF (through Typst), HTML and Word out. It is pure, so it runs in the
  command line, the desktop app and the web build (wasm). OWD pins it by git tag, `v0.1.0`, as it pins
  `otd` and `arc-rules`. Never a path or a branch.
- **A new crate, `owd-report`, builds them.** It reads a design (the document text and the DEM bytes,
  as the drawing builder does), writes an ORD document and its assets, and calls ORD to render. It is
  pure: no file, clock or network. The command line, the app shell and the web build call it.
- **ORD does the layout. OWD writes no typesetting.** OWD supplies its document classes in a register,
  the content, and the figures. ORD's default template is the OWD look, and OWD uses it unchanged.
- **Figures come from the drawing builder.** The key map, the long section and the cross sections are the
  SVG of the sheets `owd draw` makes (spec 0004), so a report and a sheet never disagree.
- **A report computes nothing.** Every number in it is read from the document's stage results, the drawing
  builder or the rule engine's trace, as the app reads them. A report of a stale stage says so.

## Considered

- **The builder in `owd-format`.** Rejected: ORD brings Typst, which is large. `owd-format` is in every
  build, the Python wheel too, and most of them do not need reports.
- **OWD writes HTML and the app prints it to PDF.** Rejected: no Word, no PDF from the command line, and
  each output would differ from the others.
- **The private Python document tool (#3's plan).** Rejected for the reasons above.

## Consequences

- Building the command line, the desktop app and the web build compiles Typst. Build time and the binary
  size grow. The web build's wasm grows the most (ORD's own wasm package is about 39 MB); spec 0014
  measures it and sets a budget.
- An ORD fix reaches OWD only by a new tag and a pin change in a pull request.
- A report's layout is ORD's. A layout change OWD needs is an ORD issue first.

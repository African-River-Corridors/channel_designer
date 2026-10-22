# Spec 0014: Reports

Status: **approved by the maintainer, 2026-10-22,** in the interview on #3. Decision: ADR 0012 (reports
through the Open Report Designer, in their own crate). Builds on spec 0004 (drawings), spec 0007 (the
screening outputs) and spec 0012 (start from a reach). Issue #3 closes when ticket 7 merges.

## Problem statement

OWD's answers come out as tables in the app, CSV and GeoJSON exports (`owd export`) and drawing sheets
(`owd draw`). Nobody can hand a decision-maker one document that says what the river is, which vessels
fit, where the river falls short, what the works cost in quantities, and what data the answer rests on.

- An engineer who wants to check a result must gather the CSV, the sheets and `owd trace` by hand.
- Nothing carries document control: a revision, a date, and who prepared, checked and approved it.
- The limits of a result (an assumed depth, a lowered mask threshold, low-confidence widths, the plan line
  the radius is read on) are in the study file and the worked-example page, not next to the numbers.
- A tester or developer who reports a problem cannot say exactly what ran, with which versions and inputs.

## Solution

`owd report` and a **Report** button in the app make four reports, each as PDF, HTML and Word, through
the Open Report Designer (ORD) v0.1.0:

1. **Screening report**, for decision-makers, with a **rule-trace appendix** for engineers.
2. **Quantity schedule**: dredge and dry cut by stretch, and the cut face.
3. **Section pack**: cross sections at chainages and the setting-out table, from the drawing sheets.
4. **Run report**, for testers and developers: what ran, the versions, and the input hashes.

Every report has document control, the screening notice on every page, and a "Data and limits" box on its
first page. The look is ORD's default template, which is the OWD look.

## User stories

### Decision-maker

1. As a decision-maker, I want one report that says which vessel classes can use the river, as it is or
   after works, so that I can decide whether to go on.
2. As a decision-maker, I want a key map and the long section in the report, so that I can see the reach
   and its hard places without the app.
3. As a decision-maker, I want the works and their quantities in the report, so that I see what the
   answer costs.
4. As a decision-maker, I want the limits of the data on the first page, so that I do not over-trust a
   screening number.
5. As a decision-maker, I want "Screening — not for construction" on every page, so that no page is read
   alone as a design.

### River engineer

6. As a river engineer, I want the design basis with the governing Rule, its citation and its status, so
   that I can quote each number with its source.
7. As a river engineer, I want the constraint stretches with their kind, chainages, deficit and works, so
   that I can check the hard places one by one.
8. As a river engineer, I want the rule trace as an appendix, so that I can audit the verdict without the
   command line.
9. As a river engineer, I want a quantity schedule of dredge and dry cut by stretch, and the tallest cut
   face, so that I can price the works.
10. As a river engineer, I want a section pack of cross sections and the setting-out table, so that a
    surveyor and a drafter can take the line further.
11. As a river engineer, I want each report as PDF, HTML and Word, so that I can issue it, publish it or
    edit it.
12. As a river engineer, I want to set the revision, date, status and who prepared, checked and approved
    the report, so that it goes through our document control.

### Tester and developer

13. As a tester, I want a run report with the OWD version, the versions of the libraries that made the
    result, every stage with its status and settings, and the hash of every input, so that I can say
    exactly what I tested.
14. As a developer, I want the same report from the same design on every platform and every run, so that a
    report is a reproducible artefact.

### App user

15. As an app user, I want a Report button that makes the same reports as the command line, on the
    desktop and on the web, so that I do not need the command line.

## Implementation decisions

### The crate (ADR 0012)

- A new crate, `crates/owd-report`, in the workspace and its default members. It depends on `owd-format`
  and on `ord` pinned by git tag `v0.1.0` in `[workspace.dependencies]`.
- It is pure: no file, clock or network. Its input is the design text, the DEM bytes when the design has
  one, and a request (the kind, the format, the design line option, document control). Its output is the
  rendered bytes, the ORD document, and the warnings.
- It computes nothing. It reads the document's stage results through `owd_format::design::Session`, the
  rule trace through `Session::rule_trace`, and the sheets through `owd_format::drawing`.
- It passes OWD's own ORD register: four classes, all on ORD's `standard` lifecycle, with number prefix
  `OWD`.

| Kind (`--kind`) | Class id | Name on the cover | Number | Contents |
|---|---|---|---|---|
| `screening` | `screening-report` | Screening report | `OWD-<design>-SR` | on |
| `quantities` | `quantity-schedule` | Quantity schedule | `OWD-<design>-QS` | off |
| `sections` | `section-pack` | Section pack | `OWD-<design>-SP` | on |
| `run` | `run-report` | Run report | `OWD-<design>-RR` | off |

`<design>` is the same design code the drawing numbers use (spec 0004).

### What every report has

- **Document control.** `revision` (default `A`), `date` (the command line gives today; the core never
  reads a clock), `status` (default `draft`), `prepared`, `checked`, `approved`, and `history` with one
  entry for the current revision. ORD's checks apply: a status needs its sign-offs (ORD004), and the
  revision's form fits the status (ORD005). An ORD finding fails the command with ORD's message.
- **The screening notice** "Screening — not for construction" (`SCREENING_NOTICE`) as ORD's `notice`, which
  prints on the cover and in the footer of every page. A design whose rules are all unverified adds the
  rules notice to the cover summary.
- **Organisation.** None by default; `--organisation` sets it.
- **"Data and limits" box** at the top of the first section. One item per fact, read from the document:

  | Item | From |
  |---|---|
  | Reach | the river name, the start and end, and the length |
  | Coordinate system | the project CRS |
  | Bank lines | their source, the seed, the mask threshold, and whether the edge fit ran |
  | Low-confidence width | the stretches flagged, with the threshold |
  | DEM | its source and its hash |
  | Water level | its source and the percentile |
  | Depth | the bed model: depth, season, and "assumed, not measured" |
  | Plan line | the line the river radius is read on (`centreline` or `fairway`) |
  | Rule Sets | each set, version and set input, and "every Rule unverified" while that holds |
  | Stale stages | each stage that is stale or not run |

  A fact the document does not hold prints "not recorded". The report never fills a gap with a default it
  did not use. Provisional defaults are marked "provisional". The box's notes list the licence notices
  (`Session::notices`).

### 1. Screening report

Sections, in order:

1. **The reach.** The Data and limits box, the river, the chainage range, and the key map (§ Figures).
2. **Data sources and their limits.** One table row per input: role, source, date, notice, and the
   provisional mark (from the provenance records). Then the limits the stages record: water-mask
   stretches, edge-fit flags, and depth stretches on single stations.
3. **What fits.** The verdict per vessel class and rule set from the fits answer, the largest class that
   fits, and the counts of constraint stretches by kind. Then the verdict of each design line option (A),
   with the count and names of blocking Rules not evaluated.
4. **Constraint stretches.** A summary table by kind (count, total length, largest deficit, works), and a
   table of the stretches of the chosen option (kind, start and end chainage, required, found, deficit,
   works, dredge, dry cut). Over 200 stretches, the table holds the 200 with the largest deficit and says
   how many it leaves out; the full list is in the quantity schedule and `owd export --stretches`.
5. **Design basis.** B: one row per quantity, with value, unit, governing Rule, set and version, citation,
   origin and status.
6. **Works and quantities.** D per option: dredge, dry cut, total, tallest cut face, and failed checks.
7. **Long section** (§ Figures).

**Appendix A, Rule trace.** E, rebuilt by `Session::rule_trace`. The rows of the channel and reach objects
in full. For the stations of the chosen option, every row whose outcome is not a pass, and the count of
passing rows per Rule. The appendix states the trace's hash, and that `owd trace --json` gives every row.

A design with no fits answer, or with no option, leaves that part out and says so in one line.

### 2. Quantity schedule

- Per option: one table of the constraint stretches with dredge and dry cut inside each, sorted by start
  chainage, with a total row. The totals equal the volumes stage's totals; a stretch quantity that the
  stage does not hold prints "not computed".
- The cut face: the tallest face of the option and its chainage, and the tallest face inside each stretch
  (from the volumes stage and its section walk).
- Units: m³ to the nearest whole number with thousands separators, chainages in km to 3 places.

### 3. Section pack

- The cross-section gallery sheets (spec 0004), one figure per sheet, in sheet order, at full text width.
- The setting-out table as a typed table (chainage, easting, northing, bearing, local radius, and the bend
  rows), from `drawing::setting_out_table`, at the drawing's stated precision.
- The typical section sheet, first.
- Sheet numbers in each caption, so a figure can be traced to its sheet.

### 4. Run report

- **Versions.** OWD, ORD, `otd`, `arc-rules`, each selected Rule Set with its version and content hash,
  the design's `format_version` and `engine_version`.
- **Stages.** Each stage: status, the engine version that ran it, its settings, and its `depends_on` hashes.
- **Inputs.** Each input: role, source, and SHA-256. The DEM's stored hash and the hash of the bytes given
  to the report, and whether they match.
- **Hashes of the results.** The document's canonical hash, the rule trace's hash, and each sheet's
  content hash.
- **Request.** What asked for the report (the command line with its arguments, or the app and its
  platform), passed in by the caller.

### Figures

- **Key map:** the plan overview sheet (spec 0004) as ORD `map` blocks, with the CRS, the sheet's scale and
  the attribution the sheet prints.
- **Long section:** the long-section sheet of the chosen option.
- **Cross sections:** the gallery sheets.
- Each is the SVG that `drawing::render` writes for `owd draw`, so a figure and its sheet are the same
  drawing. A figure whose sheet cannot be made (no DEM, no option) is left out with a one-line note.

### Command line

```
owd report <DOC> --kind screening|quantities|sections|run|all --format pdf|html|docx|all
           [--option NAME] [-o DIR] [--revision A] [--date YYYY-MM-DD] [--status draft]
           [--prepared NAME] [--checked NAME] [--approved NAME] [--organisation NAME] [--json]
```

- Writes `<number in lower case>.<pdf|html|docx>` into `DIR` (default: beside the design).
- `--json` prints the files written, the document numbers and the warnings.
- The DEM is read as `owd draw` reads it.
- `--option` defaults to the first option, as `owd draw` does.

### App

- A **Report** button beside the Sheets view's export, on the desktop and the web. It asks for the kind
  and the format, and writes the files as the Sheets view does.
- The report comes from `owd-report` through the shell (`owd-shell`), as `SheetsJob` does for the sheets,
  so the app and the command line give the same bytes. The app computes nothing.
- Document control in the app: revision, status and prepared, with the defaults above. Checked and
  approved are command-line flags only in this spec.

### Web build

- `owd-web` links `owd-report`. The ticket measures the wasm size before and after, and states both in the
  PR. If it grows by more than 50 MB, the ticket stops and reports to the maintainer before it merges.

## Testing decisions

- **A good test goes through a public seam:** `owd report … --json`, the `owd-report` API, or the shell's
  report job. Never ORD's private layout.
- **Reports are read back.** A test renders HTML and reads the text, and renders PDF and reads it with a
  second reader (as ORD's tests do), and checks that the numbers in the report equal the numbers in the
  document and the exports (`owd export --csv`, `--stretches`, `owd trace --json`). A report never has a
  number of its own.
- **The ORD document is checked.** Every report's ORD document passes `ord::check` with no errors, for
  both examples.
- **Determinism.** The same design and request give the same bytes twice, in each format.
- **Document control.** A status without its sign-offs fails with ORD004; `--status approved` with all
  three sign-offs and a number revision succeeds.
- **Data and limits.** A design with no bed model says "not recorded" for the depth; a design with one
  says "assumed, not measured". A test proves each item can fail: a document with the item changed gives
  a different box.
- **Test cases.** The synthetic example (`examples/synthetic-river.owd`) and the web example
  (`examples/web/river-example.owd` with its DEM) in CI. The Sassandra study (`examples/sassandra/run.sh`,
  network, about 20 minutes) is the acceptance run on a real river: its four reports in three formats are
  attached to the last PR, and `run.sh` gains the `owd report` step.
- **App.** One desktop smoke test and one web test see the Report button write a PDF.
- Fixed numbers in tests carry labels (`build-process.md`). Most are `property`: report value equals
  document value.

## Out of scope

- Templates other than ORD's default, and ARC's private brand pack.
- Python bindings for reports (`owd.report`): a later ticket.
- Reports of more than one design (a comparison of studies).
- Editing the report's text in OWD. The Word output is the editable copy.
- Checked and approved in the app.

## Tickets

Built in order, one PR each, test first. Each is a sub-issue of the spec issue, with native "blocked by"
links.

1. **`owd-report` and the screening report's text.** Pin ORD `v0.1.0`; the crate; the register; document
   control; the notice; the Data and limits box; screening report sections 1 to 6 as text and tables;
   `owd report --kind screening` in three formats; tests on both CI examples.
2. **Figures and the rule-trace appendix.** The key map, the long section, and Appendix A. Blocked by 1.
3. **Quantity schedule.** `--kind quantities`. Blocked by 1.
4. **Section pack.** `--kind sections`. Blocked by 1.
5. **Run report.** `--kind run`, and `--kind all`. Blocked by 1.
6. **App: the Report button** on the desktop and the web, through the shell; the wasm size measured.
   Blocked by 2, 3, 4 and 5.
7. **Acceptance: the Sassandra study.** `run.sh` gains `owd report`; the four reports in three formats
   from the study attached to the PR; the README and the website's docs page name the command; #3 closed.
   Blocked by 6.

## Risks

- **Size.** Typst makes the binaries larger, the wasm most of all (ADR 0012). Ticket 6 measures it.
- **Sheet figures on A4.** The drawing sheets are larger than A4. At text width some text on a sheet may be
  small. The HTML and the Word output keep the vector SVG, so a reader can zoom. A report-sized sheet style
  is a later change if the maintainer asks.
- **Long reaches.** A 144 km study has about 1,000 stretches per class. The screening report caps its
  stretch table (section 4) and the trace appendix lists only rows that do not pass; the schedule holds
  every stretch.
- **Word.** ORD's Word output has been opened in LibreOffice only. Ticket 7 opens one in Microsoft Word
  and records the result.

# Reports

> **Screening — not for construction.** Every report says so on every page.

OWD writes four reports of a design, each as PDF, HTML and Word, through the
[Open Report Designer](https://github.com/African-River-Corridors/open-report-designer) (ORD), the report engine OWD
pins by tag. The decision is [ADR 0012](adr/0012-reports-through-ord-in-their-own-crate.md); the spec is
[spec 0014](specs/0014-reports.md).

| Kind | For | What it holds |
|---|---|---|
| `screening` | decision-makers | the reach and its data, what fits (per vessel class and per design line option), the constraint stretches, the design basis, the works and quantities, a key map and the long section; Appendix A is the Rule Trace |
| `quantities` | engineers | for each option, the totals and the tallest cut face, and the dredge, dry cut and tallest face inside each constraint stretch |
| `sections` | engineers, surveyors | the typical section, every cross-section sheet, and the setting-out table with its bends |
| `run` | testers, developers | what asked for the report, the versions, each stage, each input's hash, and the hashes of the results |

## Make them

```bash
owd report design.owd --kind all --format all -o reports/
```

- `--kind` is `screening` (the default), `quantities`, `sections`, `run` or `all`.
- `--format` is `pdf` (the default), `html`, `docx` or `all`.
- `--option NAME` picks the design line option (default: the first).
- Files are named after the document number in lower case, for example `owd-my-river-sr.pdf`.

## What every report has

- **Document control:** revision (default `A`), date (default today), status (default `draft`), and who prepared,
  checked and approved it (`--prepared`, `--checked`, `--approved`). A status needs its sign-offs: `for-checking`
  needs a preparer, `approved` needs all three and a number revision. ORD refuses a report that breaks this.
- **The screening notice** on the cover and in the footer of every page.
- **A "Data and limits" box** on the first page: the reach, the coordinate system, where the banks came from and
  the mask threshold, low-confidence width, the DEM, the water level, the assumed depth, the plan line, the Rule
  Sets (and that every Rule is unverified), and any stage that is stale or not run. A fact the design does not
  hold says "not recorded".

## Limits

- A report computes nothing. It prints what the design's stage results hold, so a stale stage gives a stale
  report, and the box says which stages are stale.
- The figures are the drawing sheets `owd draw` makes. The sheets are larger than A4, so their text is small in the
  PDF; the HTML and Word keep the vector drawing.
- A design with no DEM or no design line option still gives every report: the parts that need them say why they
  are left out.

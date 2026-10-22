# Channel rules: Rule Sets, reaches and screening

OWD sizes a channel from **Rule Sets**: versioned data files of Rules, each citing the clause, table and
page it comes from. The rules engine is `arc-rules`, shared with open-vessel-concept-drawings. The design
is in spec 0007 and ADR 0008; the words are in the [glossary](../../GLOSSARY.md).

Every result is a screening result, not a design for construction. Every Rule is **unverified** until a
second reader checks it against its source ([how a Rule is written and checked](codification.md)).

## Choose the Rule Sets

```
owd rules                       # the Rule Sets OWD knows
owd rules show pianc-wg141      # one set, with each Rule, its citation and its status
owd rules --measures            # the quantities a Rule may test
```

OWD ships three sets:

| Set | What it holds | Set input |
|---|---|---|
| `pianc-wg141` | PIANC WG 141 (2019): bottom width, minimum bend radius, design depth and bend widening on a free-flowing river. Origin `guideline`. | `se_quality`: `A`, `B` or `C` (the Safety and Ease quality) |
| `gb50139` | The Chinese standard GB 50139 as WG 141 Appendix A.3 summarises it. Origin `national-practice`: read in a summary, so never verified from it. | `range_end`: `permissive` or `strict` |
| `owd` | OWD's own checks (a width above zero, a depth that covers the draught, the WG 141 1.2 × draught floor as a warning). Always selected. | — |

A new design selects `pianc-wg141` at quality C, marked `default`. Choose your own with `owd size`:

```
owd size design.owd --vessel CEMT_IV --rule-set pianc-wg141 --set-input se_quality=B
```

Each choice is recorded as `typed`, so you can see later which choices you made and which OWD made.

### Two sets at once: the strictest governs

Select a second set when a client asks for both standards:

```
owd size design.owd --vessel CEMT_IV \
  --rule-set pianc-wg141 --rule-set gb50139 \
  --set-input se_quality=B --set-input range_end=strict
```

Every Rule of every selected set applies, and no Rule loosens another. For each quantity the **strictest**
Rule governs: the largest required width, radius and depth. So the channel can take one set's width and the
other set's radius, and adding a set never makes a quantity smaller. The design basis (below) names the
Rule that governs each quantity.

### A Rule Set of your own

Write a Rule Set file (the format is `crates/owd-core/rules/rule-set.schema.json`; copy a built-in set as a
start) and load it at run time:

```
owd size design.owd --rule-set-file my-standard.json --rule-set my-standard --vessel CEMT_IV
owd run design.owd --rule-set-file my-standard.json
```

A file with the id and version of a loaded set but other content is refused: a changed set needs a new
version. The document records each selected set's content hash. When a set changes, the sizing stage and
every stage after it go stale.

## Rules that apply on part of the river: `owd waterway`

Some Rules apply only on one kind of waterway. GB 50139 lets the minimum radius drop (2 × L for a pushed
convoy, 3 × L for a motor vessel) on a **canal** section, where the extra width and visibility allow it.
Mark the reaches of the river by chainage along its centreline:

```
owd waterway add design.owd --from 0 --to 2000 --waterway canal --claim-reduced-radius
owd waterway list design.owd
owd waterway remove design.owd --from 0
```

- `--waterway` is `canal`, `impounded-river` or `free-flowing-river`. Chainage you don't mark is a
  free-flowing river.
- `--claim-reduced-radius` is your claim, recorded as typed. OWD accepts it on a canal reach only.
- Each station of a design line takes the inputs of the reach it falls in, and the Rule Trace shows them.
- The alignment takes one minimum radius: the largest governing radius of any reach, so the line meets
  every reach's Rule.

The old `CN-reduced` choice is now `gb50139` with `range_end=strict` on a claimed canal reach.

## Screening outputs

After the volumes stage, the document holds four answers for each design line option:

- **Verdict**: `navigable-as-is` (the river already fits), `navigable-with-works` (it fits after the works
  listed in the constraint stretches), or `not-navigable` (somewhere no works make the line fit, or the line
  is tighter than its minimum radius). The verdict also gives the count and names of any blocking Rules
  that were not evaluated, so "passes" never means more than it is.
- **Design basis**: each quantity (bottom width, minimum radius per waterway kind, design depth, widening at
  the minimum radius) with its value, the Rule that governs it, its set, citation, origin and status, and the
  other Rules of its Requirement.
- **Constraint stretches**: each run of stations where the river as found falls short of a governing
  limit. A stretch is one of these kinds:
  - `width`: the navigable water is too narrow (works: `bank-cut`);
  - `depth`: the water over the bed at the design line is too shallow (works: `dredge`);
  - `radius`: the river bends too tightly (works: `cut-through`);
  - `infeasible`: no works make the line fit (works: `none-possible`).

  Each stretch gives its chainages, the governing Rule, the required value and the river's own value at the
  worst station, the deficit, and the dredge and dry cut inside the stretch.
- **Quantities**: dredge, dry cut and tallest face, as before.

The desktop and web app show the verdict per option, the design basis in the inputs panel, and the
stretches in a table and on the map.

### Export the stretches

```
owd export design.owd --csv --stretches > stretches.csv
owd export design.owd --geojson --stretches --wgs84 > stretches.geojson
```

The CSV starts with the screening notice, the design basis and the verdicts as `#` lines, then one row per
stretch. The GeoJSON draws each stretch as the part of its design line between its ends. Both carry the
notice that the result is a screening result and that the Rules are unverified.

### The full Rule Trace: `owd trace`

```
owd trace design.owd                         # one station per reach
owd trace design.owd --option rolling-circle --json
```

The trace lists every Rule of every selected set at every object, with its value, limit, margin, outcome,
whether it governs, and its status. OWD rebuilds it from the document; it is never stored, so a long line
keeps a small file. If a set's content no longer matches the hash the document recorded, `owd trace` refuses
rather than show a trace of other rules.

## Document format 3

Spec 0007 changed the document to **format version 3**: the sizing settings hold the selected Rule Sets
and set inputs instead of a ruleset name. OWD 0.3.0 reads version 3 only and migrates no older file. To
rebuild a design made with an earlier OWD:

```
owd new design.owd --title "My river"
owd import design.owd --left-bank left.geojson --right-bank right.geojson --dem dem.tif --water-level 38
owd size design.owd --vessel CEMT_IV --rule-set pianc-wg141 --set-input se_quality=B
owd align design.owd --method rolling-circle
owd volumes design.owd
```

Then run the other stages you used (`owd optimise`, `owd draw`) as before.

The old names map like this:

| Old ruleset | Now |
|---|---|
| `PIANC-A`, `PIANC-B`, `PIANC-C` | `--rule-set pianc-wg141 --set-input se_quality=A` (or `B`, `C`) |
| `CN` | `--rule-set gb50139 --set-input range_end=permissive` |
| `CN-conservative` | `--rule-set gb50139 --set-input range_end=strict` |
| `CN-reduced` | `owd waterway add … --waterway canal --claim-reduced-radius`, then `--rule-set gb50139 --set-input range_end=strict` |

The numbers are the same as before for the same choices.

In Python, size by selection:

```python
from owd.rules import channel_shape, design_basis

shape = channel_shape("CEMT_IV", draught_m=2.5, rule_sets=["pianc-wg141"],
                      set_inputs={"se_quality": "B"})
rows = design_basis("CEMT_IV", 2.5, rule_sets=["pianc-wg141"], set_inputs={"se_quality": "B"})
```

## Width-method plugins

The width methods behind `size_channel` (`register_width_method`) are design assumptions, not verified
rules, and no document stage uses them. A package that adds a width method declares it under the
`owd.width_methods` entry-point group:

```toml
[project.entry-points."owd.width_methods"]
my_method = "my_package.widths:MY_METHOD"
```

The old group `channel_designer.width_methods` is no longer read.

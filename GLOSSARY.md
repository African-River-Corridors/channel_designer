# Open Waterway Designer (OWD)

Screening-level design of navigation channels in inland rivers: size a channel for a vessel to a
named standard, find a design line the vessel can sail, and measure the cut and dredge it needs.

## Language

### The product

**Open Waterway Designer (OWD)**:
The tool: this repository, its Python package, its command line and its desktop app. "Inland waterway" is
the term PIANC WG 141 uses for the rivers and canals a vessel navigates.
_Avoid_: Open Channel Designer ("open channel" means a canal or drain in hydraulics)

**Code names**:
The names of the parts match the product: repo `open-waterway-designer`, Python import `owd`, PyPI
`open-waterway-designer`, crates `owd-core`, `owd-format`, `owd-cli`, `owd-py` and `owd-app`, command
`owd`, file extension `.owd`.

**Former names**:
Before 2026-10-21 the tool was `channel_designer`: import `channel_designer`, command `cdesign`, file
extension `.cdesign`, crates `cd-core`, `cd-format`, `cd-cli`, `cd-py` and `cd-app`, and the app "Channel
Designer". For one release the import, the command and the extension still work (the import warns). The
crate names were never published and have no alias.
_Avoid_: using a former name in new text

### The design

**Channel design**:
One screening study of one river reach for one vessel: its inputs, the choices made, and the
results of each stage. The unit a user opens, saves (as a `.owd` file) and shares.
_Avoid_: project, scenario, run

**Stage**:
One step of a channel design that turns earlier results into new ones: sizing, alignment,
optimisation, volumes, drawings. A stage is **stale** when anything it depends on has changed
since its results were made.
_Avoid_: step, phase, layer (in user-facing text)

### Lines

**River centreline**:
The natural line midway between the two banks of the river as it is today. An input, never designed.
_Avoid_: centerline, thalweg, natural line

**Design line**:
The axis of the navigation channel being designed. Each method that produces one (rolling
circle, least-cost re-route, pool lattice) yields a new **design line option**.
_Avoid_: route, rolled line, sailing line, fairway (as a line)

**Alignment**:
The activity of finding a design line, and the stage that does it. Never a line itself.
_Avoid_: route (as a noun)

**Water level profile**:
The water level along the river centreline, as points of river chainage and level, read linearly between
points. A design line's station takes its level by projecting onto the river centreline (ADR 0006).

**Navigable water**:
The part of the water a design line may use: the water polygon shrunk by half the channel's
straight bottom width plus a clearance from the bank, then smoothed.
_Avoid_: fairway (as an area in code), wet area

**Fairway fit**:
The step that fits a smooth design line inside navigable water between fixed poses, never tighter
than R_min where the water allows: a minimum-curvature corridor solve (ADR 0007). It is a rung of
the water-aware rolling circle and the whole of a leg join, not a method of its own.

**Infeasible stretch**:
A stretch of a design line where no line that meets R_min fits inside navigable water. It is
reported with its chainage, position and reason, and the option carries a failed check.

**Seed leg**:
One of several input lines, in order, that are joined into one design line before alignment.

**Station**:
A point on a line at which a cross-section is taken, identified by its chainage.

**Chainage**:
Distance in metres along a line from its start.
_Avoid_: stationing, KP

### Channel sizing

**Rule Set**:
A named, versioned group of Rules from one source, as a data file with a content hash
(`pianc-wg141`, `gb50139`, `owd`, or a user's own file loaded at run time). A design selects one
set or more; every selected Rule applies and the strictest governs (spec 0007, ADR 0008).
_Avoid_: ruleset, standard (alone)

**Rule**:
One requirement from one source on one Measure: a limit, a citation (clause, table, page), an
origin, an applicability and a consequence, with a verification status (`unverified` until a
second reader checks it against its source).

**Measure**:
One quantity OWD computes for one object (the channel's bottom width, a station's radius), which a
Rule tests. A Measure is code; a Rule is data.

**Requirement**:
The Rules on one Measure of one object in one direction (all lower bounds on the bottom width, say).

**Governing Rule**:
The strictest Rule of a Requirement. Its limit sizes the channel.

**Rule Trace**:
Every Rule of every selected set at every object, with its value, limit, margin, outcome and
whether it governs. Rebuilt on demand (`owd trace`), never stored.

**Reach**:
A stretch of the river, by chainage on its centreline, marked as a canal, an impounded river or a
free-flowing river, with or without the reduced-radius claim. Stations take its inputs. (Not the
study reach of `owd reach`, spec 0012.)

**Design basis**:
Each quantity of the channel with the Rule that governs it, its citation and its status.

**Constraint stretch**:
A run of stations where the river as found falls short of a governing limit (width, depth or
radius), with the works that follow (bank cut, dredge, cut-through) and their quantities.

**Verdict**:
A design line option's screening answer: navigable as it is, navigable with works, or not
navigable.

**Width method**:
A way of computing channel width from a vessel and a bend radius (`size_channel`). A design
assumption, not a verified rule: no document stage uses it.

**Vessel class**:
A named class of vessel with set dimensions, from a published classification (for example CEMT IV).

**Channel shape**:
The bottom width, minimum bend radius and design depth that the selected Rule Sets require for a
vessel class.

**Minimum bend radius** (R_min):
The tightest radius the design line may turn at: the largest governing radius of any reach.
_Avoid_: turning radius

### Earthworks

**Cut model**:
A named rule for the cross-section of the works: how the channel is cut below water and how the
bank face is cut above it (Cut Model A, Cut Model B).

**Dredge**:
Material removed from below the water level.

**Dry cut**:
Material removed from above the water level.
_Avoid_: excavation (when dredge is meant too)

**Pool**:
A reach of river held at one water level by the weir at its downstream end.

**Weir cascade**:
The sequence of weirs, and the pool level each sets, along a channel design.

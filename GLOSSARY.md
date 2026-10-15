# channel_designer

Screening-level design of navigation channels in inland rivers: size a channel for a vessel to a
named standard, find a design line the vessel can sail, and measure the cut and dredge it needs.

## Language

### The design

**Channel design**:
One screening study of one river reach for one vessel: its inputs, the choices made, and the
results of each stage. The unit a user opens, saves and shares.
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

**Station**:
A point on a line at which a cross-section is taken, identified by its chainage.

**Chainage**:
Distance in metres along a line from its start.
_Avoid_: stationing, KP

### Channel sizing

**Ruleset**:
A named channel-design standard (for example PIANC quality B, or GB 50139) whose coefficients
each cite a clause, table and page.
_Avoid_: standard (alone), rule set

**Width method**:
A way of computing channel width from a vessel and a bend radius that is not just a ruleset's
coefficients.

**Vessel class**:
A named class of vessel with set dimensions, from a published classification (for example CEMT IV).

**Channel shape**:
The bottom width, minimum bend radius and design depth that a ruleset requires for a vessel class.

**Minimum bend radius** (R_min):
The tightest radius the design line may turn at, as the ruleset requires for the vessel.
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

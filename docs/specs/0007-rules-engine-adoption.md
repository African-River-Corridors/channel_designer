# Spec 0007: Channel rules on the shared rules engine

Status: **approved by the maintainer, 2026-10-21.** ADR 0008 holds the decision. The interview record is
`docs/design/2026-10-21-rules-engine-grill.md`. Every build ticket waits for an OVCD release tag that
contains `arc-rules` (OVCD #463).

## Problem statement

OWD sizes a channel from one **Ruleset**: a struct of coefficients for PIANC-A, PIANC-B or PIANC-C, or
CN, CN-conservative or CN-reduced. A river engineer screening a river for navigation cannot:

- see which rule set each dimension, or whether anyone has checked that rule against its source;
- combine two standards, or add a standard of their own without Rust;
- apply a rule to part of a line only (the reduced Chinese radius is written for canal sections, but
  today it is a whole-line choice, with a warning in its notes);
- see, in one place, where the river falls short of the rules and what works that costs.

The tool also claims "every coefficient cites its clause", but nothing checks it, and a GB 50139 number
read in a PIANC summary looks the same as a PIANC recommendation.

## Solution

OWD moves its channel rules onto `arc-rules`, the domain-free rules engine shared with OVCD.

- The rules become data: Rule Sets `pianc-wg141`, `gb50139` and `owd`, each Rule with its citation,
  origin and verification status.
- A design selects one set or more. Every selected Rule applies, and the strictest governs, per quantity
  and per station.
- A reach of the river carries its waterway kind, so a Rule can apply on canal reaches only.
- Every design gets four outputs that answer the screening questions: a **verdict**, a **design basis**,
  **constraint stretches** and the **quantities**. The full **Rule Trace** is available on demand.
- Today's numbers do not move. A parity check proves it, then the old `Ruleset` code goes.

This ships in OWD 0.3.0. The sets carry version `0.3.0`.

## User stories

### River engineer

1. As a river engineer, I want to choose a Rule Set and its inputs (for PIANC, the Safety and Ease
   quality), so that the channel is sized to a standard I can name.
2. As a river engineer, I want a new design to start from PIANC WG 141 quality C, marked as a default,
   so that I get a channel at once and can see that I did not choose it.
3. As a river engineer, I want to select a second Rule Set, so that the channel meets both standards
   where a client asks for that.
4. As a river engineer, I want each dimension of the design basis to name the Rule that governs it,
   with its citation and status, so that I can quote the number with its source.
5. As a river engineer, I want to see where the governing Rule changes along the line, so that I know
   which standard drives the channel in each place.
6. As a river engineer, I want to mark a reach of the river as a canal, impounded river or free-flowing
   river, so that Rules written for one kind apply only there.
7. As a river engineer, I want to claim the reduced GB 50139 radius on a canal reach only, and have the
   claim recorded, so that I never quote a whole-river result under a canal clause.
8. As a river engineer, I want a verdict for each design line option (navigable as it is, navigable
   with works, or not navigable), so that I can screen options quickly.
9. As a river engineer, I want a list of constraint stretches (where the river is too narrow, too
   shallow or bends too tightly for the rules, by how much, and the works that follow), so that I see
   the hard places and why.
10. As a river engineer, I want the dredge and dry cut inside each constraint stretch, so that I know
    what each hard place costs in quantities.
11. As a river engineer, I want the constraint stretches as GeoJSON and CSV, so that I can map and
    tabulate them.
12. As a river engineer, I want every passing verdict to say how many blocking Rules were not evaluated,
    and name them, so that I never read "passes" as "meets every rule".
13. As a river engineer, I want every output to say that all Rules are unverified until a second reader
    checks them, so that I do not over-trust a screening number.
14. As a river engineer, I want GB 50139 Rules marked as national practice, read in a PIANC summary, so
    that I do not mistake them for a reading of the standard.
15. As a river engineer, I want to load a Rule Set file of my own at run time, so that I can screen to
    a standard OWD does not ship.
16. As a river engineer, I want a stage to go stale when its Rule Sets change, so that a number in the
    file always matches the rules it names.
17. As a river engineer, I want the same channel as today when I make the same choices, so that earlier
    screening results stay valid.

### Reviewer or expert

18. As a reviewer, I want each Rule's limit, coefficient, clause and page in a readable data file, so
    that I can check it against the guideline without reading Rust.
19. As a reviewer, I want `owd rules show <set>` to print every Rule with its status, so that I know
    what has and has not been checked.
20. As a reviewer, I want the full Rule Trace for a design (each Rule at each station, with value,
    limit, margin and outcome), so that I can audit any result.
21. As an expert, I want to verify a Rule by recording the edition and locator I read, so that its
    status changes from unverified to verified.

### App user

22. As an app user, I want to pick the Rule Set and quality in the inputs panel, so that I do not need
    the CLI.
23. As an app user, I want the verdict, the design basis and the constraint stretches as tables, so
    that I can read the result at a glance.
24. As an app user, I want the constraint stretches drawn on the map, so that I can see where they are.

### Agent and maintainer

25. As an agent, I want to change a Rule by editing a data file in a pull request, so that a rule change
    never needs a code change.
26. As a maintainer, I want OWD to use the same engine as OVCD, so that its proofs and tests cover both
    tools.
27. As a maintainer, I want the old `Ruleset` code gone once parity holds, so that there is one path.

## Implementation decisions

### The engine

- `arc-rules` and `ovcd-core` come from the same OVCD release tag. OWD adds no engine logic: no
  comparison, no strictness, no trace building of its own.
- OWD registers its object types, its Measures, its applicability vocabulary and its formula shapes with
  the engine, as OVCD does.
- Rule Set files live in `owd-format` beside a JSON Schema, are built into every binary, and load
  through the same path as a user's file.

### Objects

| Object type | How many | Inputs it carries |
|---|---|---|
| `channel` | one per design | the vessel (LOA, beam, draught, formation, class), the set inputs |
| `reach` | one or more, by chainage on the river centreline | `waterway`, `reduced_radius_claim` |
| `station` | one per station of a design line option | inherited from the reach its projection onto the river centreline falls in |

- Reaches attach to the river centreline, as the water level does (ADR 0006). With no reach marked, the
  whole river is one reach of kind `free-flowing-river` with no claim.
- `waterway` is `canal`, `impounded-river` or `free-flowing-river`. `reduced_radius_claim` is `yes` or
  `no`, typed by the user.
- Object-scoped inputs need the request on OVCD #463. If the tag lacks them, the reach ticket stops and
  reports; it does not fall back to one object type per reach kind.

### Rule Sets at 0.3.0

Each set encodes today's coefficients bit for bit. Every Rule is `unverified`.

| Old ruleset | New selection |
|---|---|
| PIANC-A, PIANC-B, PIANC-C | `pianc-wg141` with `se_quality` A, B or C |
| CN | `gb50139` with `range_end` permissive |
| CN-conservative | `gb50139` with `range_end` strict |
| CN-reduced | `gb50139` with `range_end` strict, on a `canal` reach with `reduced_radius_claim: yes` |

- **`pianc-wg141`** (origin `guideline`): bottom width `k_w × B` (Table 5.7, p109, with the 3.0 B
  minimum for quality C); minimum radius `k_r × LOA` (Table 5.7); depth `max(k_d × T, 1.2 × T)`
  (5.3.4.3, p108); bend widening `min(c_C × L² / R, L)` (5.3.3, p101; c_C from Table 5.7 fn 3 and
  Table E.5, p261).
- **`gb50139`** (origin `national-practice`, citation "GB 50139 as summarised in WG 141 Appendix A.3,
  p162–164"): straight width from Table A.2 when the vessel matches an entry, else WG 141 Table 5.1's
  China-River ratio 2.3 × B; bend widening `L² / (2R + B)` (A.3.3); depth `T + clearance`, where
  classes I to III take the strict end of the A.3.2 range (today's fixed choice, whatever the range
  end) and class IV and up take a fallback add-on of 0.3 m (permissive) or 0.5 m (strict), with
  today's `warn` text; minimum radius 3L (pushed convoy) or 4L (motor vessel), and the reduced 2L or 3L
  Rule on claimed canal reaches. The A.3.2 swept path, whose bank increment follows `range_end`, stays
  as a `note` Rule that compares it with the width used. So at 0.3.0 the range end changes only the
  class IV-and-up depth and that note; this is what today's code does, and it becomes a finding.
- **`owd`** (OWD Checks, never shown as a published rule): straight width above zero; depth covers
  draught; bed below datum (`warn`); depth below 1.2 × T under a set that allows it (`warn`, today's
  text, origin `guideline`, citing WG 141 5.3.4.3).
- Each width, radius and depth Rule is a lower bound with `sizes: true`.
- **Formula shapes** are Rust, registered by OWD, with coefficients in the file. Each keeps the order of
  operations of today's code, so no bit of a result moves: `multiple` (k × x), `ratio-with-floor`,
  `plus-clearance`, `capped-widening` (min(c × L² / R, L)), `gb-widening` (L² / (2R + B)), and
  `gb-straight-width` (the Table A.2 match within today's tolerance, else the ratio).
- **Versions.** The set files carry `0.3.0` and may change in place until OWD 0.3.0 is tagged. Before
  1.0.0 a shipped version is not frozen: OVCD's released-sets gate does not apply. The content hash is
  always recorded. 1.0.0 needs a production-ready set with expert review.
- **Differences from the text are findings, not fixes.** The canal values of Table 5.2, two-way widths,
  c_C by vessel class and state from Table E.5, the R/L ≥ 5 validity range of the bend-widening formula,
  and the WG 141 worked example's 3.0 B reading each get a finding issue, linked from the Rule. Each
  lands later as a design change.

### Selection and composition

- The document stores the selection: each set with id, version, content hash and `source` (`default` or
  `typed`), and each set input with its value and source. `owd` is always selected.
- A new design selects `pianc-wg141` with `se_quality` C, all marked `default`.
- All Rules of all selected sets apply. The governing limit of each Requirement sizes the channel.
  The same Rules check the result at each station.
- **Alignment and reaches.** The alignment takes one R_min: the largest governing R_min of any reach
  on the line. So the line meets every reach's Rule. The station checks still evaluate each reach's own
  Rule. A radius that varies along the alignment is a later spec. With one canal reach covering the
  whole line and the claim made, this gives today's CN-reduced result.
- Bend widening at a station uses the station's local radius on the chord (spec 0005's one radius
  reading). The governing width can change from station to station.

### Document format

- `format_version` 3. The sizing settings hold the selection and the set inputs instead of
  `ruleset_id`. The river holds the reaches.
- A version 2 file is refused with a message that names the version and says to rebuild the design.
  There is no migration, and no backward compatibility.
- A stage's dependency hash includes the selected sets' content hashes and the reaches, so a rule
  change marks it stale.

### Outputs

The document stores A to D for each design line option. E is never stored.

- **A. Verdict.** `navigable-as-is` (no constraint stretch, no failing row), `navigable-with-works`
  (constraint stretches with works, no failing row) or `not-navigable` (any failing row, for example an
  infeasible stretch). It carries the count and names of blocking Rules not evaluated, and "Screening —
  not for construction".
- **B. Design basis.** One row per quantity (bottom width on a straight, minimum bend radius, design
  depth, widening at R_min), per reach kind present: value, unit, governing Rule id, set and version,
  citation, origin, verification status, and the other Rules of its Requirement.
- **C. Constraint stretches.** A run of stations where the river as found falls short of a governing
  limit: kind (`width`, `depth`, `radius` or `infeasible`), start and end chainage and position, the
  governing Rule, the required value, the river's own value at the worst station and that station's
  chainage, the deficit, the works that follow (`bank-cut`, `dredge`, `cut-through` or none possible),
  and the dredge and dry cut inside the stretch. The river's values come from the existing stages: the
  width of navigable water, the depth from the water level and the bed at the design line, and the radius
  of the river centreline. Infeasible stretches are spec 0005's.
- **D. Quantities.** As today.
- **E. Rule Trace.** `owd trace` rebuilds the full trace (every Rule of every selected set, at every
  object, with value, limit, margin, outcome, governing flag and status) from the document and the stored
  hashes. A stored hash that does not match the engine's set is refused, not silently re-evaluated.
- Exports: C as GeoJSON lines and CSV, both with the screening notice. B and A in the CSV header block.

### CLI

- `owd size` takes `--rule-set <id>` (repeatable), `--set-input <name>=<value>` and `--rule-set-file
  <path>` (repeatable, loaded at run time).
- `owd reach` adds, lists and removes reaches by chainage, with `--waterway` and `--claim-reduced-radius`.
- `owd rules` lists sets; `owd rules show <id>` prints one with its Rules and status; `owd rules
  --measures` prints the Measures.
- `owd trace <file>` prints E, as a table or `--json`, for one option or all.

### App

- Inputs panel: the Rule Set picker (one set by default, add a second), the set inputs, and the
  selection's `default` or `typed` mark. Reaches are edited with the CLI in this spec.
- Results: the verdict, the design basis table and the constraint stretches table, with tags in words
  and shapes (not colour only). Constraint stretches are drawn on the map. The app computes nothing:
  every field comes from the document.

### Python

- The thin wrapper exposes sizing by selection and the four outputs. `register_ruleset` and the
  `Ruleset` class go. The width method registry, the `pianc` width method with its Report 121 build-up,
  and `ChannelDesignParams` stay, and their docs say they are design assumptions, not verified rules.

## Testing decisions

- **A good test goes through a public seam:** `owd … --json`, the design session, the rules loader, or
  the Python wrapper. Never the private layout of a module.
- **Parity is the main test of tickets 1 to 4.** For every old ruleset × every built-in vessel class ×
  a spread of draughts, the new path gives the old `ChannelShape` bit for bit, through the old
  `Ruleset` code kept for this check only. Every fixture design gives identical stage results. The
  design-impact check reports none.
- **Formula-shape properties** (property tests on random vessels and radii): bottom width never rises
  as R rises; widening ≤ L for `capped-widening`; every width above zero; quality A ≥ B ≥ C on every
  quantity; adding a set never makes a design basis value smaller; one set selected twice changes
  nothing.
- **Sourced boundary tests.** Each evaluated Rule gets cases just inside and just outside its limit,
  keyed by rule id, written from the Rule's data, not by hand.
- **Golden case.** The WG 141 worked example (5.3.3, p104) as a test, recording the known divergence
  (the example's totals add up against 3.0 B, not the 3.2 B it quotes).
- **Constraint stretches.** A property test: every station where the river falls short of a governing
  limit lies inside exactly one stretch of that kind, and no station outside a stretch falls short.
  Synthetic fixtures (a straight trapezoid river with one narrow throat, one shallow bar and one tight
  bend) give known stretches.
- **Trace rebuild.** `owd trace` on a saved document gives the same rows on every run and platform, and
  refuses a document whose stored hash differs.
- **Proofs.** The kernel's four Lean theorems come with `arc-rules` (OVCD ADR 0014). A sizing theorem
  (the governing limit meets every sizing Rule of its Requirement; adding a Rule never lowers it) is
  requested in OVCD's Lean model. OWD runs no Lean, and nothing Lean is a required check.
- Prior art: OVCD's `rule_set`, `rule_trace` and `fit_rules` tests; this repo's parity runner and
  design-impact tool.

## Out of scope

- The canal values, two-way widths, Table E.5 c_C by state, and the validity range: findings, later.
- A radius that varies along the alignment by reach.
- A vessel-class sweep ("the largest class this river takes").
- Encoding the width method registry, the Report 121 build-up or `ChannelDesignParams` as Rule Sets.
- Verifying any Rule (a second reader with the text). The codification process is in this spec; the
  readings are not.
- Editing reaches in the app.
- Moving `arc-rules` to its own repo.

## Tickets

Each build ticket is blocked by an OVCD release tag that contains `arc-rules`. That block is stated in
each ticket, because a native link cannot cross repos. Tickets 1 to 5 and 8 are built in order; 6 and 7
can run beside them once their blockers close.

1. **`pianc-wg141` through the engine.** Pin `arc-rules`; register the `channel` object, its Measures and
   the PIANC formula shapes; the `pianc-wg141` set; sizing through the engine behind the old
   `channel_shape`; parity for PIANC-A, PIANC-B and PIANC-C; the codification process note.
2. **`gb50139` through the engine.** The GB shapes and set, `range_end`, origin `national-practice`;
   parity for CN and CN-conservative. Blocked by 1.
3. **Selection in the document.** `format_version` 3; the selection with sources; the default; two sets
   at once; the CLI flags and `owd rules`; loading a set file at run time; stale on set hash. Blocked by 2.
4. **Reaches.** The `reach` and `station` objects, object-scoped `waterway` and the claim; the reduced
   radius Rule; the alignment's one R_min; parity for CN-reduced. Blocked by 3.
5. **Screening outputs.** A, B and C in the document; C as GeoJSON and CSV; `owd trace`. Blocked by 4.
6. **App.** The inputs panel and the three result tables; C on the map. Blocked by 5.
7. **Proofed checks.** Shape property tests, sourced boundary tests, the golden case. Blocked by 2.
8. **Contract.** Remove `Ruleset`, `register_ruleset` and the parity-only path; update GLOSSARY,
   README and CHANGELOG; mark the library width methods as design assumptions. Blocked by 5, 6 and 7.

## Risks

- **OVCD's API moves under OWD.** Schema 1.x is additive only; the pin is a tag. If the tag lacks
  object-scoped inputs or the two origins, tickets 1 and 4 stop and report.
- **Results move.** Parity and the design-impact check guard every step; any moved number stops the
  build.
- **Formula shapes bloat the kernel.** They live in OWD, registered with the engine, never in
  `arc-rules`.
- **Release coupling.** OWD 0.3.0 waits on an OVCD tag.
- **Users over-trust "Rule Set".** Every output says the Rules are unverified, and GB 50139 Rules say
  they are read in a summary.

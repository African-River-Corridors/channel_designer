---
status: accepted
---
# Channel rules are Rule Sets on the shared rules engine, and the strictest Rule governs

Today a **Ruleset** is a fixed struct of coefficients: PIANC-A, PIANC-B and PIANC-C; CN, CN-conservative
and CN-reduced. A channel design picks one, and its numbers give the channel shape. The coefficients
cite a clause, table and page, but nothing records which rules a result was checked against, which rule
set a number, or whether anyone has read the cited page. The width, the radius and the depth sit in
code (`k × beam`, `k × LOA`, `ratio × draught`), next to `if family == "china"` branches. A user cannot
add a standard without Rust, and cannot combine two.

The sister project, open-vessel-concept-drawings (OVCD), already has a rules engine that solves this:
versioned, content-hashed Rule Sets as data, every applicable Rule applies, the strictest governs, a
Rule Trace on every result, and Lean proofs of four properties of the composition kernel (OVCD ADR
0013, ADR 0014, spec 0004). Its evaluator is domain-free. OVCD is extracting it into a domain-free
crate, `arc-rules`, with no change to any OVCD result (OVCD #463).

## Decision

- **OWD uses `arc-rules`.** It pins the crate by an OVCD release tag, as it pins `ovcd-core` (ADR 0001).
  Both crates come from the same tag. OWD builds no second engine.
- **The OVCD terms apply.** A **Rule** is data: one requirement from one source on one **Measure**,
  with a citation, an Applicability, a Criterion and a consequence. A **Measure** is code: one quantity
  OWD computes for one object. A **Rule Set** is a named, versioned group of Rules from one source.
  The **Rule Trace** lists every Rule of every selected set. "Ruleset" goes out of the vocabulary.
- **One Rule Set per source, not per tier.** `pianc-wg141` holds the WG 141 rules, and the Safety and
  Ease quality (A, B or C) is an input that its Rules apply on. `gb50139` holds the Chinese standard as
  WG 141 Appendix A.3 summarises it, and the end of each range the standard states (permissive or
  strict) is an input. The `owd` set holds OWD's own checks (depth covers draught, a width above zero,
  a bed below datum) and is always selected. Two tiers of one source in one selection would be pointless
  under strictest governs, because the higher tier always wins.
- **Channel rules size the channel.** Each width, radius and depth Rule is a lower bound with
  `sizes: true`. The channel takes the **governing** limit of each Requirement: the largest required
  width, radius or depth among the selected Rules. The same Rules then check the result, so a design
  line tighter than R_min still fails.
- **Strictest governs, per quantity and per station.** All selected Rules apply; none loosens another.
  So the governing Rule can differ by quantity (one set's width, another set's radius) and by station
  (two bend-widening formulas cross over as the radius changes). The channel then matches no single
  standard on its own, and the outputs say which Rule governs where. A design usually selects one set;
  combining sets is allowed, not expected.
- **Per-reach applicability, never a looser set.** A reach carries `waterway`: `canal`,
  `impounded-river` or `free-flowing-river`. Its stations inherit it, and a Rule may apply on it. GB
  50139 lets the radius drop where the section is a canal and the extra width and visibility allow it.
  That is one Rule in `gb50139`, which applies only on a canal reach where the user claims the
  reduction; the claim is a typed input and shows in the Rule Trace. CN-reduced as a whole-line set goes.
  This needs object-scoped inputs in `arc-rules` (requested on OVCD #463).
- **Honest status.** Every OWD Rule starts `unverified`. WG 141 Rules have origin `guideline`, because
  WG 141 says it is not a standard. GB 50139 Rules have origin `national-practice`: WG 141's Appendix
  A.3 reports them, and OWD does not hold GB 50139 itself. A GB Rule checked only against Appendix A.3
  is never `verified`. Both origins are requested on OVCD #463.
- **The default selection is `pianc-wg141` at quality C**, stored as `default`, not `typed`.
- **The outputs answer the screening questions.** The document stores a verdict; a design basis (each
  quantity with its governing Rule, citation and status); constraint stretches (where the river fails a
  Rule, by how much, and the works that follow); and the quantities, as today. The full Rule Trace, one
  row per Rule per station, is rebuilt on demand from the document and the stored set hashes. It is
  never stored, so a long line stays small.
- **Versions before 1.0.0 move.** The sets ship as version `0.3.0` with OWD 0.3.0, and change in place
  until then. Before 1.0.0, a shipped set version is not frozen, and the document format makes no
  promise to old files: `format_version` 3 refuses a version 2 file with a clear message. 1.0.0 is kept
  for sets that are production ready and have had expert review. The content hash is always recorded,
  so a stage still goes stale when its rules change.

## Considered

- **Keep one chosen standard, with coefficients as data.** A smaller change, but it does not combine
  standards, and it keeps a second engine beside OVCD's.
- **Keep six sets, one per tier.** Simple to map, but a selection of two tiers of one source does
  nothing, and nothing tells the user.
- **A reach-level choice of set** (one set on the river, another on the cut-throughs). It needs a
  looser set to replace a stricter one on part of the line, and the engine never lets a Rule loosen
  another. Applicability on a reach input does the same job inside the rules.
- **One object type per reach kind** (`canal-station`, `river-station`), as OVCD does for tank roles.
  No engine change, but every Measure and Rule would exist twice.
- **Store the full trace in the document.** About 100,000 rows on a 50 km line at 10 m stations.
  A folded trace was also considered; the screening outputs carry what a reader needs, and the full
  trace can always be rebuilt.
- **Our own engine.** It would duplicate the Lean-proven kernel and drift from it.

## Consequences

- OWD results with today's choices do not change. Each old ruleset maps to a set and its inputs
  (PIANC-B is `pianc-wg141` with quality B; CN-conservative is `gb50139` with the strict end). A
  parity check proves the new path gives the old numbers bit for bit, then the old `Ruleset` code goes.
- Where OWD's numbers differ from the WG 141 text (canal values, two-way widths, c_C by vessel state,
  the validity range of the bend-widening formula), each difference becomes a finding and lands later
  as a design change.
- Adding a set to a design makes its channel equal or larger, never smaller, and a stage that used the
  old selection goes stale.
- OWD releases wait on OVCD tags. If OVCD's cadence blocks OWD, `arc-rules` moves to its own repo.
- The width method registry, the PIANC Report 121 build-up and `ChannelDesignParams` stay as library
  API. No document stage uses them. They are design assumptions, not verified rules, and the docs say
  so. Report 121 is a maritime approach-channel guideline.
- The kernel's proofs come with `arc-rules`. A sizing theorem (the sized channel meets every sizing
  Rule, and adding a set never makes it smaller) is added to its Lean model. OWD's own formula shapes
  get property tests, and each Rule gets sourced boundary tests.
- The design and the build plan are in `docs/specs/0007-rules-engine-adoption.md`. The decisions are in
  `docs/design/2026-10-21-rules-engine-grill.md`.

## References

- OVCD ADR 0013 (rules are versioned Rule Sets), ADR 0014 (the Lean model), spec 0004 (the rules
  engine), issue #463 (the `arc-rules` extraction).
- PIANC InCom WG 141 (2019), *Design Guidelines for Inland Waterway Dimensions*: Tables 5.2 and 5.7,
  5.3.3, 5.3.4.3, Table E.5, Appendix A.3.

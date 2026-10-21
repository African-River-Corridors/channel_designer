# Rules engine adoption: design decisions (grill, 2026-10-21)

The record of a design interview between the maintainer and the Systems Architect agent, for ADR 0008
and spec 0007. The terms are in `GLOSSARY.md` and in OVCD's spec 0004. This note keeps every answer,
so the spec can be checked against it.

## Why

The maintainer wants a robust rules engine with proven checks on its logic. OVCD has one. The
maintainer decided that both tools share it: OVCD extracts a domain-free `arc-rules` crate (OVCD #463),
and OWD pins it by tag. OWD's rule sets become selectable, and the strictest governs. These were
decided before the interview and were not re-opened.

## Decisions

| # | Question | Answer |
|---|---|---|
| Q1 | Rule Set grain | One set per source. `pianc-wg141` takes the Safety and Ease quality as an input; `gb50139` takes the range end (permissive or strict) as an input. The six old rulesets map onto these. |
| Q2 | Status of GB 50139 rules | Origin `national-practice`, cited as "GB 50139 as summarised in WG 141 Appendix A.3", `unverified` until someone reads GB 50139 itself. WG 141 Rules have origin `guideline`. Both origins are requested from `arc-rules` (OVCD #463). |
| Q3 | Per-reach applicability | Object-scoped inputs in `arc-rules` (requested on OVCD #463). A reach carries `waterway` (`canal`, `impounded-river`, `free-flowing-river`); stations inherit it. The reduced GB radius is one Rule that applies on a canal reach where the user claims the reduction. |
| Q4 | Strictest by quantity and by station | Supported, as the engine requires. In practice a design usually selects one set. |
| Q5a | Which outputs | A verdict; a design basis (each quantity with its governing Rule); constraint stretches (where the river fails a Rule, by how much, and the works that follow); the quantities; and the full Rule Trace on demand. The screening questions are: can this vessel use this river, where are the hard places and why, and how much work does it take. |
| Q5b | What the document stores | The verdict, design basis, constraint stretches and quantities. The full trace is rebuilt on demand by `owd trace` and never stored. |
| Q5c | Vessel-class sweep | A later spec. Spec 0007 makes each output one row per design, so a sweep is a table over designs. |
| Q6 | First encoding | The sets first encode today's numbers bit for bit. Each difference from the WG 141 text becomes a finding and lands later. |
| Q7 | Width methods outside the document | Out of scope. The `pianc` width method, the Report 121 build-up and `ChannelDesignParams` stay as library API, documented as design assumptions, not verified rules. |
| Q8 | Proofed checks | The kernel's Lean proofs come with `arc-rules`. A sizing theorem is added to its Lean model. OWD adds property tests on its formula shapes, sourced boundary tests per Rule, the WG 141 worked example as a golden case, and the parity check. Lean stays out of the required build. |
| Q9 | Default selection | `pianc-wg141` at quality C, stored as `default`. |
| Q10 | Versions | OWD's next release is 0.2.1. This work ships in OWD 0.3.0, and the sets carry version `0.3.0`, changing in place until that release. 1.0.0 is kept for sets that are production ready and have had expert input. |
| Q11 | Old files and the old API | No backward compatibility: the work is changing fast and no one relies on old files yet. `format_version` 3 refuses a version 2 file. The `Ruleset` code stays only as a parity check during the build, then goes. |

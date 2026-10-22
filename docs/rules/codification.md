# Codifying a channel rule

How a person or an agent turns a clause of a channel guideline into a Rule that OWD can load. The
design is in `docs/specs/0007-rules-engine-adoption.md` and ADR 0008. The engine is `arc-rules`,
shared with open-vessel-concept-drawings (OVCD); this page follows OVCD's own codification page and
changes only what is particular to channels.

You do not need to read Rust. You need the guideline open at the right clause, a text editor, and
this page.

## Where the rules live

- One file per Rule Set version: `crates/owd-core/rules/<set id>/<version>.json`, beside
  `rule-set.schema.json` (the engine's schema 1.x, copied from the pinned tag).
- Every built-in file is built into every binary and loads through the same loader as a user's file
  (`owd_core::rules::engine::built_in`). A file the loader refuses stops the build's tests.
- The Measures a Rule may test, the inputs it may apply on, and the formula shapes are OWD code, listed in
  `owd_core::rules::engine` (`MEASURES`, `SE_QUALITY`, `SHAPES`). A new one needs Rust and its own test.

## Who does what

| Role | Does | May not |
|---|---|---|
| **Codifier** (a person or an agent) | Finds the clause, writes the Rule, adds the tests, opens the pull request | Set `verified` on a Rule they wrote |
| **Second reader** (a person or an agent who did not write the Rule) | Reads the cited clause in the cited edition and checks the record field by field | Change the record in the same review without saying so |
| **Maintainer** | Merges; decides findings; decides any change to a released set | — |

The record names a role (`maintainer`, `expert-reviewer` or `agent`), never a person: this repository is
public.

## The steps

1. **Source and edition.** Use the published text and record its edition. Cite only what you have read.
   A paywalled text (PIANC reports are) gets its numbers, tables and clause numbers recorded, never its
   prose. If the file is in a rule-source library, add it to the set's `sources` with its SHA-256.
2. **Clause and rule id.** One Rule per requirement. The id is `<family>/<locator>/<slug>`, the locator
   with spaces removed: `pianc-wg141/Table5.7/bottom-width-c`, `pianc-wg141/5.3.4.3/design-depth-a`. The
   loader checks the id against `source.locator`. An id is never reused for another requirement.
3. **Measure and Criterion.** Pick the Measure (`channel.bottom-width`, `channel.min-bend-radius`,
   `channel.design-depth`, `channel.bend-widening`, and the vessel Measures a formula reads). "At least" is
   `ge`. A formula names a registered shape and gives its coefficients as printed, in the text's order of
   operations.
4. **Applicability and citation.** The conditions in the text, as inputs: `se_quality` for the WG 141
   Safety and Ease quality. The citation: document, edition, locator, page, access, and `also` for
   the footnotes and tables the requirement reads with.
5. **Origin and status.** `guideline` for WG 141, which says it is not a standard. `national-practice` for
   a national standard known only through another text, such as GB 50139 as WG 141 Appendix A.3
   summarises it. Such a Rule is never `verified` from the summary alone. `codified` names the role, the
   date and the pull request. `verified` stays `false` until step 7.
6. **Tests.** A sourced test on both sides of each limit, worked from the text by hand. A Rule that
   replaces old code also keeps the parity test: the new path gives the old numbers bit for bit.
7. **Second reading.** The second reader opens the cited edition at the cited page and checks the
   locator, the numbers and units, the comparison, the formula's form, the applicability and the
   consequence. Then they fill `verified` (`by`, `date`, `edition_held`, `locator_read`, `review`), or write
   what is wrong.
8. **Differences from the text are findings, not fixes.** Where today's number differs from the text,
   file a finding issue, link it from the Rule (`finding`), and change the number later, as its own design
   change with a design-impact entry (a fragment `changelog.d/<pr>.design-impact.md`).

## Versions

The sets carry the version of the OWD release they ship in (`0.3.0`) and may change in place until that
release is tagged. Before 1.0.0 a shipped set version is not frozen. The content hash is always recorded,
so a stage that used a set goes stale when the set changes. 1.0.0 is kept for sets that are production
ready and have had expert review.

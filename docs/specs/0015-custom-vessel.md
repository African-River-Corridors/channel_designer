# Spec 0015: Custom vessel

Status: **approved by the maintainer, 2026-10-22,** issue #355, in an interview (notes:
`docs/design/2026-10-22-custom-vessel-grill.md`). Builds on spec 0002 (story 6: "a vessel class (or my own
vessel dimensions)"), spec 0007 (the rules engine) and spec 0014 (reports).

## Problem statement

OWD sizes a channel only for a vessel class it knows. `owd size --vessel` and the app's vessel picker take
one of 12 class ids, and nothing else.

- A designer often has a vessel of their own: a concept vessel, or a fleet vessel that no catalogue class
  matches. A wide self-propelled barge, for example, is wider than every CEMT motor vessel. Today they must
  pick the nearest class and override the draught, so the width and the radius are sized for a different
  vessel.
- The 12 classes are a hand-written list from `ovcd-core` v0.2.2. The shared vessel catalogue (OVCD v0.2.4,
  which `arc-rules` already pins) holds 292 classes, but OWD cannot pick them. `CEMT-VIb-motor`
  (140 × 15 m), for example, is refused as "unknown vessel standard".
- Spec 0002 story 6 asked for "my own vessel dimensions". No ticket built it.

## Solution

1. **The full catalogue.** OWD moves `ovcd-core` to v0.2.4, the version `arc-rules` already pins. Every
   catalogue class can be picked. The app's picker groups the classes by family and has a search box.
2. **A custom vessel.** A study can hold one custom vessel: a name, the length overall, the beam, the
   draught and the formation, with optional fields. It is sized, checked, aligned, drawn and reported like a
   catalogue class, and every output labels it as custom.
3. **In the CLI,** `owd size --custom-vessel NAME --loa … --beam … --draught … --formation …`.
4. **In the app,** "Custom vessel…" in the vessel picker shows the input fields, filled from the class that
   was selected.
5. **File format 0.4.** The format version becomes a string, `"0.4"`. Version 1.0 waits for an external
   review of the format.

## User stories

1. As a river engineer, I want to enter my own vessel's length, beam, draught and formation, so that the
   channel is sized for the vessel I will actually use.
2. As a river engineer, I want to start a custom vessel from the class I have selected, so that I change
   only the numbers that differ.
3. As a river engineer, I want a clear message when a dimension cannot be right (zero, negative, or a beam
   not less than the length), so that I do not size a channel for a typing error.
4. As a river engineer, I want a warning, not a refusal, when my vessel is outside every catalogue class,
   so that an unusual vessel can still be screened.
5. As a river engineer, I want a draught deeper than the river to be allowed, so that the dredging it needs
   shows as works.
6. As a decision-maker, I want every output to say the vessel is custom and give its dimensions, so that I
   never mistake it for a standard class.
7. As a river engineer, I want `owd fits` to include my custom vessel, so that I see whether it fits the
   river as it is, beside the standard classes.
8. As a river engineer, I want to pick any class in the shared catalogue, so that I can compare against the
   class nearest my vessel.
9. As a river engineer using GB 50139, I want to give my custom vessel a GB class, so that the GB width and
   depth checks can run.
10. As a river engineer, I want to record the air draught and deadweight with my vessel, so that a later
    bridge check can use them.
11. As a river engineer, I want my studies saved in the old format to open, so that nothing I made is lost.

## Implementation decisions

### Ticket 0: the full catalogue

- `ovcd-core` moves from git tag `v0.2.2` to `v0.2.4` in `crates/owd-core/Cargo.toml`, the tag `arc-rules`
  already uses. OWD then links one OVCD version.
- **Ids.** A study that holds an old id (`CEMT_Va`, `CN_I_motor` and the other 10) still loads: the lookup
  goes through OVCD's `LEGACY_IDS`. A study saved by the new OWD stores the catalogue id (`CEMT-Va`). The
  rule trace and the results show the catalogue id.
- **Pickable classes.** The picker and `owd size --vessel` offer every class that has a length, a beam and
  a draught (OVCD's `pickable()`). A class without them is refused with a message that names the missing
  field.
- **Design values.** As today: the length is the long end of its range, the beam is one value, and the
  draught is the middle of its range unless the user gives one.
- **`owd fits`** checks the same 12 classes as today by default (by their catalogue ids), plus the study's
  custom vessel. `--all` checks every pickable class.
- **The app picker** groups classes by family (CEMT, RWS, CN, the regional families) and has a search box
  that matches the id, the name and the family.
- **Design impact.** No design result may move for a study that uses one of the 12 classes. A test proves
  it on both CI examples. If a class's figures differ between v0.2.2 and v0.2.4, the PR carries a
  `design-impact` fragment that says what moved and why.

### Ticket 1: the custom vessel in the core and the CLI

**The fields.**

| Field | Required | Unit | Used by |
|---|---|---|---|
| `name` | yes | text, 1–60 characters | labels |
| `loa_m` | yes | m | radius, bend widening (PIANC); GB 50139 |
| `beam_m` | yes | m | width (PIANC); GB 50139 |
| `draught_m` | yes | m | depth (all three sets) |
| `formation` | yes | `motor` or `convoy` | GB 50139 |
| `n_abreast`, `n_in_line` | for `convoy` only | count | GB 50139 |
| `gb_class` | no | `I`–`VII`, or none | GB 50139 width and depth |
| `air_draught_m` | no | m | stored only (a later bridge check) |
| `deadweight_t` | no | t | stored only |

- For a convoy, `loa_m` and `beam_m` are those of the whole convoy, as in the catalogue.
- **GB class.** GB 50139 cannot derive a class for a vessel it does not know (its tonnage and
  typical-vessel tables are keyed on catalogue classes). A typed `gb_class` is used. With none, the
  `3.0.1/*-needs-class` rules mark the width and depth "not evaluated", as they do today for an unknown
  class. The custom vessel is sized as `special` (Appendix A).
- **Where it lives.** In the study only: `stages.sizing.settings.custom_vessel`, an object with the fields
  above. When it is set, `vessel_standard_id` is `null`. There is no vessel file in this spec.

**Checks.**

- **Refused:** a dimension that is zero, negative or not a number; a beam not less than the length; an
  empty name; a convoy with no `n_abreast` or `n_in_line`. The message names the field.
- **Warned, and the run goes on:** a vessel outside the length, beam and draught envelope of every pickable
  class; a length-to-beam ratio below 2.5 or above 15. The warning shows in the CLI output, the app, and
  the report's Data and limits box.
- **Never refused:** a draught deeper than the river. It shows as dredging, as for any class.

**The CLI.**

```
owd size DOC --custom-vessel "My barge" --loa 140 --beam 29 --draught 4.7 --formation motor
owd size DOC --custom-vessel "My convoy" --loa 190 --beam 22.8 --draught 3.5 --formation convoy \
    --abreast 2 --in-line 2 --gb-class III
```

- `--custom-vessel NAME` takes `--loa`, `--beam`, `--draught` and `--formation`; `--abreast` and
  `--in-line` for a convoy; and `--gb-class`, `--air-draught` and `--deadweight` as options.
  `--custom-vessel` and `--vessel` are mutually exclusive.
- `owd size` saves the vessel into the study. Later commands (`align`, `volumes`, `fits`, `draw`,
  `report`, `export`, `trace`) read it from there, with no flags.
- `owd size DOC --vessel ID` replaces a custom vessel with a catalogue class.

**Labels.** Everywhere a class id appears, a custom vessel shows as `Custom: <name>, L × B × T` (for
example `Custom: My barge, 140 × 29 × 4.7 m`): the `size` and `fits` output, the rule trace, the drawing's
typical section, the reports' design basis, and the exports. The report's Data and limits box gains a
"Vessel" item: "user-entered dimensions, not a catalogue class", and any envelope warning.

**`owd fits`.** When the study has a custom vessel, `fits` adds it as one more row, after the classes.

**File format 0.4.**

- `format_version` becomes a string, `"0.4"`. The schema's `format_version` takes `"0.4"`.
- A file with the number `3` reads as format 0.3. It upgrades to 0.4 in memory and is saved as 0.4. Nothing
  else in it changes. A test proves a 0.3 study opens and gives the same results.
- A 0.4 file opened by an older OWD gets its existing "made by a newer OWD" message.
- Format 1.0 waits for an external review of the format. Until then a format change raises the minor
  number.

### Ticket 2: the app

- The vessel picker (`SizingForm.svelte`) lists the catalogue by family, with a search box (from ticket 0),
  and a last entry, **Custom vessel…**.
- Choosing Custom shows the fields of ticket 1, filled from the class that was selected: its name with
  " (custom)", its length, beam, draught and formation. The user edits them.
- The **GB class** field shows only when GB 50139 is among the selected rule sets.
- **Air draught** and **deadweight** are under "More" and are optional.
- A refused value shows next to its field, and the Size button stays off until it is fixed. A warning
  shows under the form and does not block.
- The shell (`owd-shell`) gains the custom vessel in its sizing call, so the desktop and the web app share
  one path.
- The fits table's "Use this vessel" button works for the custom row too.

## Testing decisions

- **A good test goes through a public seam:** `owd size … --json`, the `owd-format` session API, the
  shell's sizing call, or the app.
- **Ticket 0.** The same results before and after for each of the 12 classes on both CI examples
  (`property`); an old id loads and saves as its catalogue id; `CEMT-VIb-motor` sizes; a class with no
  draught is refused with a message that names it; `fits` checks 12 classes by default and every pickable
  class with `--all`.
- **Ticket 1.**
  - A custom vessel with the dimensions of a catalogue class gives the same width, radius and depth as
    that class under PIANC (`property`).
  - Each refusal has a test that fails it, and each warning a test that raises it.
  - A draught deeper than the river sizes, and `volumes` shows the dredge.
  - GB 50139: with `gb_class` the width and depth are evaluated; without it they are "not evaluated".
  - The label appears in the size output, the trace, the typical section, the screening report and the
    exports.
  - `fits` has one more row with a custom vessel.
  - A format-0.3 study opens, upgrades and gives the same results; the schema accepts `"0.4"`.
- **Ticket 2.** One desktop smoke test and one web test: choose Custom vessel, see the fields filled, edit
  the beam, size, and see the label. A refused beam turns the Size button off.

## Out of scope

- A vessel file to reuse a custom vessel across studies, and a library of custom vessels in the app.
- More than one custom vessel in a study.
- Bridge clearance and air-draught checks. The air draught is stored for them.
- Adding a custom vessel to the shared OVCD catalogue.
- Format 1.0.

## Tickets

Built in order, one PR each, test first. Each is a sub-issue of the spec issue, with native "blocked by"
links.

0. **The full catalogue (#356).** `ovcd-core` to v0.2.4; legacy ids; every pickable class in `owd size` and the
   picker (grouped by family, with search); `fits --all`; no design impact for the 12 classes. Touches the
   app (the picker), so it takes the app slot.
1. **The custom vessel in the core and the CLI (#357).** The fields, the checks, `owd size --custom-vessel`, the
   labels in every output, `fits`, and format 0.4. Blocked by 0. Does not touch the app.
2. **App: Custom vessel… (#358).** The fields in the picker, through the shell, on the desktop and the web. Blocked
   by 1. Takes the app slot.

## Risks

- **Design impact from the catalogue move.** OVCD v0.2.4 may hold different figures for a class than
  v0.2.2. Ticket 0's property test catches it; any move is recorded in a `design-impact` fragment.
- **A long picker.** 292 classes is a long list. The family groups and the search box keep it usable.
- **GB 50139 for a custom vessel.** Without a GB class half its checks do not run. The output says so;
  it never guesses a class.
- **The format change.** Every tool that reads `.owd` files (the Python wheel, the web app) must accept
  `"0.4"` and the number `3`. Ticket 1 tests both readers.

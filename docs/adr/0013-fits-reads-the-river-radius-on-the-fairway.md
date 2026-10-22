---
status: accepted
---
# `fits` reads the river's radius on the fairway

The full answer of `owd fits` (spec 0012) reads the river's own radius on a plan line. Until now that
line was the river centreline, and the fairway was an opt-in (`--plan-line fairway`, #278). The
centreline is drawn from the bank lines. It wiggles at the scale of its stations, and each wiggle reads
as a tight bend. On the whole Sassandra the maintainer compared the two (2026-10-22): each class's
fairway gave far fewer radius stretches than the centreline (CEMT-Va: 315 against 1,058). The width and
depth stretches did not change.

## Decision

- **The default plan line is the fairway.** For each class, the full answer fits that class's fairway
  (ADR 0007) in the river's water, with its R_min and bottom width, and reads the river's radius on it.
  `--plan-line centreline` still reads the centreline. The command line, the desktop app and the web
  app take the same default, from `Session::run_fits_with`.
- **The answer says which line it read.** `fits.settings.plan_line` holds the line asked for.
  `fits.results.plan_line` holds the line used, and each row of a full answer holds its own. When the
  line used is not the one asked for, `plan_line_note` says why.
- **When the fairway cannot run, the answer falls back to the centreline and says so.**
  - A plan-only answer (no DEM, bed model or water level) always reads the centreline. Its
    `plan_line_note` says that the fairway runs only for the full answer.
  - In a full answer, a class whose fairway fit fails reads the centreline. Its row carries the note
    with the reason, and the answer counts the rows that fell back.
- **An answer made before this decision keeps its centreline.** It has no `plan_line` key, so it
  reads as centreline in the table and the reports. It is not marked stale. Run `owd fits` again to
  read the fairway.

The full answer takes longer, because each class fits its own line: on the Sassandra (spec 0012 run),
about 160 s on the fairway against about 45 s on the centreline.

## Considered options

- **Keep the centreline as the default.** Rejected: most of its radius stretches are the wiggle of a
  line drawn from the banks, not the river.
- **A longer chord on the centreline** (option b in #278). Not taken.
- **A vessel-independent smooth reference line.** Later (spec 0016, for 0.4): one smooth line for the
  river, and a sweep of vessel sizes on it. It may supersede this default.

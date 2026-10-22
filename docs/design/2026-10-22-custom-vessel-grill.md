# Custom vessel: interview notes (2026-10-22)

The maintainer's answers that shaped spec 0015.

| Question | Answer |
|---|---|
| Where a custom vessel lives | In the study only, for now. No vessel file, no library. |
| How the app enters it | "Custom vessel…" in the picker, with the fields filled from the selected class. |
| Checks | Refuse a value that is not positive and a beam not less than the length. Warn, but go on, for a vessel outside every catalogue class or an odd length-to-beam ratio. A draught deeper than the river is allowed: it shows the works needed. |
| Labels | "Custom: <name>, L × B × T" everywhere a class id appears; "user-entered dimensions, not a catalogue class" in the report's Data and limits box. |
| `owd fits` | The custom vessel is one more row. |
| Fields | Required: name, length, beam, draught, formation. GB class only when GB 50139 is selected. Air draught and deadweight optional, stored only. |
| The catalogue | Move `ovcd-core` to v0.2.4 first (ticket 0), so every catalogue class can be picked. |
| The picker | Grouped by family, with a search box. `fits` keeps today's 12 classes by default; `--all` checks every class. |
| File format | Change the format version to 0.4. Version 1.0 when the format has been reviewed externally. Format-3 studies still open. |
| CLI | `owd size --custom-vessel NAME --loa --beam --draught --formation`, with `--abreast`, `--in-line` and `--gb-class`. |
| Build order | Ticket 0, then ticket 1 (core and CLI, no app slot), then ticket 2 (the app). |

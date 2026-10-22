# The largest vessel: interview notes (2026-10-22)

The maintainer's answers that shaped spec 0016.

| Question | Answer |
|---|---|
| Starting point | An earlier two-step method: a smooth mid-line from the banks (pairing, Chaikin, Taubin), then a vessel-size sweep. |
| Reference line | The smooth mid-line as the seed, refitted in the water, held off the banks, pulled towards the mid-line. Not a tiny-vessel racing line, not the mid-line alone. |
| Plan line | The reference line becomes the plan line of `owd fits` (in 0.4). Release 0.3.0 ships with the fairway as the default. |
| What fits means | No blocked stretch in the reach, plus the largest vessel for each kilometre. |
| Sizes | One series: 10 × 2, 20 × 4, 30 × 6, 40 × 8 and 50 × 10 m. |
| Draught | 2 m. Plan only (width and radius). |
| Rules | PIANC WG 141 quality C by default, changeable; the strictest Rule governs. |
| Reference line settings | Clearance the larger of 5 m and 10 % of width; no minimum radius; one pull weight, tuned; low-confidence banks marked. |
| Vessel types | Single hulls only. |
| Outputs | `owd envelope` (JSON, CSV, PNG), a report section, an app panel. |
| Tickets | Reference line, then the envelope command, then the report, then the app. |

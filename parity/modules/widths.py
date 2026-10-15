"""widths: `alignment/widths.py` (river width measured across the centreline, bank to bank)."""
from __future__ import annotations

from shapely.geometry import LineString

from ..common import Rng, case as _case, frozen, rline, synth

ENTRY_POINT = "widths"
R = "initial case set (spec 0001, ticket 23)"


def adapter(fn: str, args: dict):
    w = frozen("alignment.widths")
    if fn == "resample_distances":
        return w.resample_distances(**args)
    if fn == "measure_widths":
        rows, failures = w.measure_widths(**args)
        return {"rows": rows, "failures": failures}
    raise KeyError(f"widths has no parity function {fn!r}")


def cases():
    out = []

    def c(cid, tier, fn, args):
        out.append(_case(f"{fn}/{cid}", tier, fn, args, R))

    for total, sp in [(1000.0, 50.0), (1000.0, 333.0), (10.0, 100.0), (0.0, 10.0), (999.9, 10.0)]:
        c(f"distances_{total:g}_{sp:g}", 1, "resample_distances", dict(total_length=total, spacing=sp))
    c("distances_zero_spacing", 3, "resample_distances", dict(total_length=100.0, spacing=0.0))
    cl = LineString([(0, 0), (1000, 0)])
    c("parallel_banks", 1, "measure_widths", dict(
        centreline=cl, left_bank=LineString([(0, 20), (1000, 20)]),
        right_bank=LineString([(0, -20), (1000, -20)]), spacing=50.0, cross_span=100.0))
    c("narrow_cross_span_fails", 1, "measure_widths", dict(
        centreline=cl, left_bank=LineString([(0, 20), (1000, 20)]),
        right_bank=LineString([(0, -20), (1000, -20)]), spacing=100.0, cross_span=10.0))
    c("bank_ends_early_fails", 1, "measure_widths", dict(
        centreline=cl, left_bank=LineString([(0, 20), (500, 20)]),
        right_bank=LineString([(0, -20), (1000, -20)]), spacing=100.0, cross_span=100.0))
    c("banks_cross_twice_takes_nearest", 1, "measure_widths", dict(
        centreline=cl, left_bank=LineString([(0, 20), (400, 20), (400, 60), (600, 60), (600, 20), (1000, 20)]),
        right_bank=LineString([(0, -20), (1000, -20)]), spacing=100.0, cross_span=100.0))
    c("zero_length_centreline_segment", 1, "measure_widths", dict(
        centreline=LineString([(0, 0), (0, 0), (100, 0)]), left_bank=LineString([(0, 20), (100, 20)]),
        right_bank=LineString([(0, -20), (100, -20)]), spacing=25.0, cross_span=60.0))
    for tag, kw in [("default", {}), ("wide", dict(width_m=70.0)), ("tight_meander", dict(amplitude_m=300.0, wavelength_m=800.0))]:
        rv = synth(length_m=1800.0, **kw)
        c(f"synthetic_{tag}", 1, "measure_widths", dict(
            centreline=rv["centreline"], left_bank=rv["left_bank"], right_bank=rv["right_bank"],
            spacing=100.0, cross_span=100.0))
    c("cross_span_zero", 3, "measure_widths", dict(
        centreline=cl, left_bank=LineString([(0, 20), (1000, 20)]),
        right_bank=LineString([(0, -20), (1000, -20)]), spacing=50.0, cross_span=0.0))
    c("spacing_zero", 3, "measure_widths", dict(
        centreline=cl, left_bank=LineString([(0, 20), (1000, 20)]),
        right_bank=LineString([(0, -20), (1000, -20)]), spacing=0.0, cross_span=10.0))
    rng = Rng(23010)
    for i in range(12):
        rv = synth(length_m=rng.r3(1000.0, 2400.0), wavelength_m=rng.r3(600.0, 1500.0),
                   amplitude_m=rng.r3(60.0, 280.0), width_m=rng.r3(20.0, 80.0))
        c(f"sweep_{i:02d}", 2, "measure_widths", dict(
            centreline=rv["centreline"], left_bank=rv["left_bank"], right_bank=rv["right_bank"],
            spacing=rng.r3(20.0, 150.0), cross_span=rng.r3(40.0, 150.0)))
    for i in range(8):
        c(f"distances_sweep_{i:02d}", 2, "resample_distances",
          dict(total_length=rng.r3(1.0, 5000.0), spacing=rng.r3(0.5, 500.0)))
    return out

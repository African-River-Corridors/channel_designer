"""Volumes layer on synthetic geometry: section integration, pools, footprint, raster."""
from __future__ import annotations

import numpy as np
import pytest
from shapely.geometry import LineString

from channel_designer.volumes import pools
from channel_designer.volumes.cut_model import WET_SLOPE_H_PER_V
from channel_designer.volumes.footprint import channel_footprint, compute_excavation
from channel_designer.volumes.sections import section_volumes, station_frame, trapezoid_area

L = 1000.0
HB = 15.0
DEPTH = 3.0
WL = 5.0


def _plane(z):
    return lambda xs, ys: np.full(np.shape(xs), float(z))


# --- section integration --------------------------------------------------------------------

def test_golden_flat_wet_bed_gives_the_trapezoid_volume():
    """Bed 1 m below WL everywhere, all wet: cut = trapezoid from grade up to the bed."""
    line = LineString([(0, 0), (L, 0)])
    river = LineString([(-50, 0), (L + 50, 0)]).buffer(250.0, cap_style="flat")
    v = section_volumes(line, _plane(WL - 1.0), river, WL, DEPTH, HB, station_m=25.0)
    h = DEPTH - 1.0
    area = 2 * HB * h + WET_SLOPE_H_PER_V * h * h
    n = len(v.stations)
    assert v.excav_m3 == 0.0
    assert v.dredge_m3 == pytest.approx(area * 25.0 * n, rel=0.01)


def test_property_dry_ground_above_wl_without_intrusion_cuts_only_below_wl():
    """Ground at WL+0 (not above), dry: the whole wet prism is excavation; no face above WL."""
    line = LineString([(0, 0), (L, 0)])
    river = LineString([(0, 500), (L, 500)]).buffer(10.0)         # river elsewhere
    v = section_volumes(line, _plane(WL), river, WL, DEPTH, HB, station_m=50.0)
    assert v.dredge_m3 == 0.0 and v.max_face_h_m == 0.0
    per_station = trapezoid_area(HB, DEPTH, WET_SLOPE_H_PER_V)
    assert v.excav_m3 == pytest.approx(per_station * 50.0 * len(v.stations), rel=0.01)


def test_property_hb_forms_agree():
    line = LineString([(0, 0), (L, 0)])
    river = line.buffer(200.0, cap_style="flat")
    n = len(station_frame(line, 25.0)[0])
    a = section_volumes(line, _plane(WL - 1), river, WL, DEPTH, HB)
    b = section_volumes(line, _plane(WL - 1), river, WL, DEPTH, np.full(n, HB))
    c = section_volumes(line, _plane(WL - 1), river, WL, DEPTH, lambda xy: np.full(len(xy), HB))
    assert a.total_m3 == b.total_m3 == c.total_m3
    with pytest.raises(ValueError):
        section_volumes(line, _plane(WL - 1), river, WL, DEPTH, np.full(n + 1, HB))


def test_feasibility_model_b_never_cuts_more_than_a():
    from channel_designer.synthetic import synthetic_river
    r = synthetic_river()
    seg = LineString(list(r.centreline.coords)[150:230])           # reach past the hill
    a = section_volumes(seg, r.terrain, r.polygon, r.water_level, 3.0, 15.0, model="a",
                        maxw_m=150.0)
    b = section_volumes(seg, r.terrain, r.polygon, r.water_level, 3.0, 15.0, model="b",
                        maxw_m=150.0)
    assert b.total_m3 <= a.total_m3 + 1e-6
    assert b.dredge_m3 == pytest.approx(a.dredge_m3)


def test_by_tag_splits_the_total():
    line = LineString([(0, 0), (L, 0)])
    river = line.buffer(200.0, cap_style="flat")
    v = section_volumes(line, _plane(WL - 1), river, WL, DEPTH, HB,
                        tag=lambda s: "first" if s < L / 2 else "second")
    t = v.by_tag()
    assert set(t) == {"first", "second"}
    assert sum(x["dredge_m3"] for x in t.values()) == pytest.approx(v.dredge_m3)


# --- pools ----------------------------------------------------------------------------------

def _linear_profile(slope=1e-3, length=20000.0, spacing=25.0):
    line = LineString([(0, 0), (length, 0)])
    terrain = lambda xs, ys: np.asarray(xs) * slope              # noqa: E731
    return pools.sample_bed_profile(line, terrain, spacing_m=spacing,
                                    slope_perc_window_m=500.0, slope_smooth_window_m=500.0)


def test_golden_weirs_on_a_linear_bed_are_spaced_lift_over_slope():
    prof = _linear_profile()
    weirs = pools.place_weirs(prof, weir_height_m=6.0, implantation_depth_m=2.0)
    lift = 4.0
    wls = [w.pool_water_level_m for w in weirs]
    assert np.allclose(np.diff(wls), lift)
    gaps = np.diff([w.chainage_m for w in weirs])
    assert np.allclose(gaps[1:-1], lift / 1e-3, rtol=0.05)


def test_property_water_level_and_grade_follow_the_cascade():
    prof = _linear_profile()
    weirs = pools.place_weirs_explicit(prof, [
        {"chainage_m": 5000, "pool_mwl_m": 7.0, "structure_height_m": 8},
        {"chainage_m": 12000, "pool_mwl_m": 14.0, "structure_height_m": 8},
    ])
    assert pools.water_level_at(weirs, 100.0) == 0.0
    assert pools.water_level_at(weirs, 6000.0) == 7.0
    assert pools.water_level_at(weirs, 15000.0) == 14.0
    assert pools.grade_at(weirs, 6000.0, depth_m=3.0) == 4.0
    assert pools.water_level_fn(weirs, 0.5)(10.0) == 0.5


def test_feasibility_zero_lift_is_refused():
    with pytest.raises(ValueError):
        pools.place_weirs(_linear_profile(), weir_height_m=2.0, implantation_depth_m=2.0)


# --- footprint ------------------------------------------------------------------------------

def test_golden_footprint_of_a_channel_wider_than_the_river():
    c = LineString([(0, 0), (L, 0)])
    left, right = LineString([(0, 30), (L, 30)]), LineString([(0, -30), (L, -30)])
    banks_l, banks_r = LineString([(0, 20), (L, 20)]), LineString([(0, -20), (L, -20)])
    ex = compute_excavation(left, right, banks_l, banks_r)
    assert sorted(e["bank_side"] for e in ex) == ["left", "right"]
    assert sum(e["area_m2"] for e in ex) == pytest.approx(2 * 10 * L)


def test_property_footprint_grows_with_beam():
    from channel_designer.synthetic import synthetic_river
    r = synthetic_river()
    small = channel_footprint(r.centreline, r.left_bank, r.right_bank, loa=80, beam=8, depth=3)
    big = channel_footprint(r.centreline, r.left_bank, r.right_bank, loa=80, beam=16, depth=3)
    assert big.total_excavation_area_m2 > small.total_excavation_area_m2 > 0


# --- raster (needs the [dem] extra) --------------------------------------------------------

def _write_tif(path, z, x0, y0, px):
    rasterio = pytest.importorskip("rasterio")
    from rasterio.transform import from_origin
    with rasterio.open(path, "w", driver="GTiff", height=z.shape[0], width=z.shape[1], count=1,
                       dtype="float32", crs="EPSG:3857", transform=from_origin(x0, y0, px, px),
                       nodata=-9999.0) as ds:
        ds.write(z.astype("float32"), 1)


def test_raster_terrain_bilinear_reproduces_a_plane(tmp_path):
    pytest.importorskip("rasterio")
    from channel_designer.volumes.raster import RasterTerrain
    px = 2.0
    xs = np.arange(0, 400, px) + px / 2
    ys = 200 - (np.arange(0, 200, px) + px / 2)
    X, Y = np.meshgrid(xs, ys)
    _write_tif(tmp_path / "plane.tif", 0.01 * X + 0.02 * Y, 0.0, 200.0, px)
    with RasterTerrain(tmp_path / "plane.tif") as t:
        qx = np.array([50.3, 120.7, 333.3]); qy = np.array([40.1, 99.9, 150.2])
        assert np.allclose(t(qx, qy), 0.01 * qx + 0.02 * qy, atol=1e-4)
        assert np.isnan(t(np.array([-50.0]), np.array([10.0])))[0]


def test_raster_chainage_volumes_match_the_section_integrator(tmp_path):
    pytest.importorskip("rasterio")
    from channel_designer.volumes.raster import compute_chainage_volumes
    px = 1.0
    xs = np.arange(-50, 1050, px) + px / 2
    ys = 300 - (np.arange(0, 600, px) + px / 2)
    X, _ = np.meshgrid(xs, ys)
    _write_tif(tmp_path / "flat.tif", np.full(X.shape, WL - 1.0), -50.0, 300.0, px)
    line = LineString([(0, 0), (L, 0)])
    river = line.buffer(250.0, cap_style="flat")
    weir = pools.Weir(index=1, chainage_m=0.0, point=None, bed_m=0.0, foundation_m=0.0,
                      crest_m=WL, pool_water_level_m=WL, pool_start_m=0.0, pool_end_m=L)
    recs = compute_chainage_volumes([weir], line, river, tmp_path / "flat.tif",
                                    navigable_depth_m=DEPTH, channel_bottom_width_m=2 * HB,
                                    side_slope_h_per_v=WET_SLOPE_H_PER_V, despike=False,
                                    chunk_m=100.0, max_corridor_half_width_m=60.0)
    raster_total = sum(r["dredge_m3"] for r in recs)
    h = DEPTH - 1.0
    analytic = (2 * HB * h + WET_SLOPE_H_PER_V * h * h) * L
    assert raster_total == pytest.approx(analytic, rel=0.03)

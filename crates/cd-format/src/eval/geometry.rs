//! The `geometry` and `synthetic` entry points (ticket 24), and the JSON form of a geometry
//! (`parity/common.py`, `encode_geom` / `decode_geom`).

use super::{EvalError, bad, f64_arg, f64_list, f64_or, float_json, list, ok, read_float};
use cd_core::geom::offset::{self, Cap};
use cd_core::geom::overlay::{self, Op};
use cd_core::geom::{GeomError, Geometry, Polygon, Pt, linear, merge, valid};
use cd_core::synthetic::{self, RiverParams};
use serde_json::{Value, json};

// --- geometry as JSON -------------------------------------------------------------------------

pub(super) fn pt(v: &Value, what: &str) -> Result<Pt, EvalError> {
    match v.as_array().map(Vec::as_slice) {
        Some([x, y, ..]) => Ok([read_float(x, what)?, read_float(y, what)?]),
        _ => Err(bad(format!("{what}: a point is [x, y]"))),
    }
}

pub(super) fn pts(v: &Value, what: &str) -> Result<Vec<Pt>, EvalError> {
    v.as_array()
        .ok_or_else(|| bad(format!("{what}: must be a list of points")))?
        .iter()
        .map(|p| pt(p, what))
        .collect()
}

fn poly_from(v: &Value, what: &str) -> Result<Polygon, EvalError> {
    let holes = match v.get("holes") {
        None => vec![],
        Some(h) => h
            .as_array()
            .ok_or_else(|| bad(format!("{what}.holes: must be a list")))?
            .iter()
            .map(|r| pts(r, what))
            .collect::<Result<_, _>>()?,
    };
    Ok(Polygon {
        exterior: pts(&v["exterior"], what)?,
        holes,
    })
}

/// `decode_geom`.
pub(super) fn geom_from(v: &Value, what: &str) -> Result<Geometry, EvalError> {
    let kind = v
        .get("geom")
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("{what}: needs `geom`")))?;
    if v.get("empty").and_then(Value::as_bool) == Some(true) {
        return Ok(Geometry::Empty(match kind {
            "Point" => "Point",
            "LineString" => "LineString",
            "Polygon" => "Polygon",
            "MultiPoint" => "MultiPoint",
            "MultiLineString" => "MultiLineString",
            "MultiPolygon" => "MultiPolygon",
            "GeometryCollection" => "GeometryCollection",
            k => return Err(bad(format!("{what}: unknown geometry kind `{k}`"))),
        }));
    }
    Ok(match kind {
        "Point" => Geometry::Point(pt(&v["coords"], what)?),
        "LineString" => Geometry::LineString(pts(&v["coords"], what)?),
        "Polygon" => Geometry::Polygon(poly_from(v, what)?),
        "MultiPoint" => Geometry::MultiPoint(pts(&v["coords"], what)?),
        "MultiLineString" => Geometry::MultiLineString(
            v["lines"]
                .as_array()
                .ok_or_else(|| bad(format!("{what}.lines")))?
                .iter()
                .map(|l| pts(l, what))
                .collect::<Result<_, _>>()?,
        ),
        "MultiPolygon" => Geometry::MultiPolygon(
            v["polygons"]
                .as_array()
                .ok_or_else(|| bad(format!("{what}.polygons")))?
                .iter()
                .map(|p| poly_from(p, what))
                .collect::<Result<_, _>>()?,
        ),
        "GeometryCollection" => Geometry::Collection(
            v["geoms"]
                .as_array()
                .ok_or_else(|| bad(format!("{what}.geoms")))?
                .iter()
                .map(|g| geom_from(g, what))
                .collect::<Result<_, _>>()?,
        ),
        k => return Err(bad(format!("{what}: unknown geometry kind `{k}`"))),
    })
}

fn pt_json(p: Pt) -> Value {
    json!([float_json(p[0]), float_json(p[1])])
}

fn pts_json(ps: &[Pt]) -> Value {
    Value::Array(ps.iter().map(|&p| pt_json(p)).collect())
}

fn poly_body(p: &Polygon) -> Value {
    json!({"exterior": pts_json(&p.exterior), "holes": p.holes.iter().map(|h| pts_json(h)).collect::<Vec<_>>()})
}

/// `encode_geom`.
pub(super) fn geom_json(g: &Geometry) -> Value {
    match g {
        Geometry::Point(p) => json!({"geom": "Point", "coords": pt_json(*p)}),
        Geometry::LineString(l) => json!({"geom": "LineString", "coords": pts_json(l)}),
        Geometry::Polygon(p) => json!({
            "geom": "Polygon",
            "exterior": pts_json(&p.exterior),
            "holes": p.holes.iter().map(|h| pts_json(h)).collect::<Vec<_>>(),
        }),
        Geometry::MultiPoint(ps) => json!({"geom": "MultiPoint", "coords": pts_json(ps)}),
        Geometry::MultiLineString(ls) => {
            json!({"geom": "MultiLineString", "lines": ls.iter().map(|l| pts_json(l)).collect::<Vec<_>>()})
        }
        Geometry::MultiPolygon(ps) => {
            json!({"geom": "MultiPolygon", "polygons": ps.iter().map(poly_body).collect::<Vec<_>>()})
        }
        Geometry::Collection(gs) => {
            json!({"geom": "GeometryCollection", "geoms": gs.iter().map(geom_json).collect::<Vec<_>>()})
        }
        // As `encode_geom` writes an empty geometry of each kind.
        Geometry::Empty("LineString") => json!({"geom": "LineString", "coords": []}),
        Geometry::Empty("Polygon") => json!({"geom": "Polygon", "exterior": [], "holes": []}),
        Geometry::Empty("MultiLineString") => json!({"geom": "MultiLineString", "lines": []}),
        Geometry::Empty("MultiPolygon") => json!({"geom": "MultiPolygon", "polygons": []}),
        Geometry::Empty(k) => json!({"geom": k, "empty": true}),
    }
}

/// shapely 2's default `quad_segs` for `buffer` and `offset_curve` (GEOS's own default is 8).
const SHAPELY_QUAD_SEGS: i32 = 16;

// --- argument helpers -------------------------------------------------------------------------

fn geom_arg(args: &Value, name: &str) -> Result<Geometry, EvalError> {
    geom_from(super::arg(args, name)?, &format!("args.{name}"))
}

/// A line argument: its points (an empty line has none).
fn line_arg(args: &Value, name: &str) -> Result<Vec<Pt>, EvalError> {
    match geom_arg(args, name)? {
        Geometry::LineString(l) => Ok(l),
        Geometry::Empty("LineString") => Ok(vec![]),
        _ => Err(bad(format!("`args.{name}` must be a LineString"))),
    }
}

fn polys_of(g: Geometry, what: &str) -> Result<Vec<Polygon>, EvalError> {
    match g {
        Geometry::Polygon(p) if p.exterior.is_empty() => Ok(vec![]),
        Geometry::Polygon(p) => Ok(vec![p]),
        Geometry::MultiPolygon(ps) => Ok(ps),
        Geometry::Empty("Polygon" | "MultiPolygon") => Ok(vec![]),
        _ => Err(bad(format!("{what} must be a polygon"))),
    }
}

fn int_or(args: &Value, name: &str, default: i32) -> Result<i32, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_i64()
            .and_then(|i| i32::try_from(i).ok())
            .ok_or_else(|| bad(format!("`args.{name}` must be an integer"))),
    }
}

fn bool_or(args: &Value, name: &str, default: bool) -> Result<bool, EvalError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| bad(format!("`args.{name}` must be true or false"))),
    }
}

fn cap_arg(args: &Value) -> Result<Cap, EvalError> {
    match args.get("cap_style") {
        None | Some(Value::Null) => Ok(Cap::Round),
        Some(Value::String(s)) => {
            Cap::from_shapely(s).ok_or_else(|| bad(format!("unknown cap_style `{s}`")))
        }
        Some(Value::Number(n)) => {
            Cap::from_shapely(&n.to_string()).ok_or_else(|| bad(format!("unknown cap_style {n}")))
        }
        _ => Err(bad("`args.cap_style` must be a name or code".into())),
    }
}

fn geom_err(e: GeomError) -> EvalError {
    bad(e.to_string())
}

fn sort_key(p: Pt) -> (i64, i64) {
    // Rounded to 1e-6 m, so a last-bit difference cannot reorder two distinct points.
    ((p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64)
}

// --- the entry points -------------------------------------------------------------------------

/// The `geometry` entry point: one shapely operation per function (`parity/modules/geometry.py`).
pub(super) fn geometry(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "line_length" => ok(float_json(linear::length(&line_arg(args, "line")?))),
        "interpolate" => {
            let line = line_arg(args, "line")?;
            let normalized = bool_or(args, "normalized", false)?;
            let out = f64_list(args, "distances")?
                .into_iter()
                .map(|d| {
                    let p = if normalized {
                        linear::interpolate_normalized(&line, d)
                    } else {
                        linear::interpolate(&line, d)
                    };
                    geom_json(&p.map_or(Geometry::Empty("Point"), Geometry::Point))
                })
                .collect();
            ok(Value::Array(out))
        }
        "project" => {
            let line = line_arg(args, "line")?;
            let len = linear::length(&line);
            let normalized = bool_or(args, "normalized", false)?;
            let out = list(args, "points")?
                .iter()
                .map(|p| {
                    let m = linear::project(&line, pt(p, "args.points")?);
                    Ok(float_json(if normalized { m / len } else { m }))
                })
                .collect::<Result<_, EvalError>>()?;
            ok(Value::Array(out))
        }
        "substring" => {
            let line = line_arg(args, "line")?;
            let (mut s, mut e) = (f64_arg(args, "start")?, f64_arg(args, "end")?);
            if bool_or(args, "normalized", false)? {
                let len = linear::length(&line);
                s *= len;
                e *= len;
            }
            ok(geom_json(&linear::substring(&line, s, e)))
        }
        "distance_points_line" => {
            let line = line_arg(args, "line")?;
            let out = list(args, "points")?
                .iter()
                .map(|p| {
                    Ok(float_json(linear::point_line_distance(
                        pt(p, "args.points")?,
                        &line,
                    )))
                })
                .collect::<Result<_, EvalError>>()?;
            ok(Value::Array(out))
        }
        "distance_points_line_vectorised" => {
            let line = line_arg(args, "line")?;
            let (xs, ys) = (f64_list(args, "xs")?, f64_list(args, "ys")?);
            ok(Value::Array(
                xs.iter()
                    .zip(&ys)
                    .map(|(&x, &y)| float_json(linear::point_line_distance([x, y], &line)))
                    .collect(),
            ))
        }
        "distance_geoms" => ok(float_json(distance(
            &geom_arg(args, "a")?,
            &geom_arg(args, "b")?,
        )?)),
        "intersection_points" => {
            let (a, b) = (line_arg(args, "a")?, line_arg(args, "b")?);
            let (mut points, lines) = linear::line_intersection(&a, &b);
            points.sort_by_key(|&p| sort_key(p));
            let mut lines: Vec<Vec<Pt>> = lines
                .into_iter()
                .map(|mut l| {
                    if sort_key(l[0]) > sort_key(l[l.len() - 1]) {
                        l.reverse();
                    }
                    l
                })
                .collect();
            lines.sort_by_key(|l| (sort_key(l[0]), sort_key(l[l.len() - 1])));
            ok(
                json!({"points": pts_json(&points), "lines": lines.iter().map(|l| pts_json(l)).collect::<Vec<_>>()}),
            )
        }
        "line_merge" => {
            let lines: Vec<Vec<Pt>> = list(args, "lines")?
                .iter()
                .map(|l| match geom_from(l, "args.lines")? {
                    Geometry::LineString(l) => Ok(l),
                    _ => Err(bad("`args.lines` must hold LineStrings".into())),
                })
                .collect::<Result<_, _>>()?;
            let mut merged = merge::line_merge(&lines);
            ok(geom_json(&match merged.len() {
                0 => Geometry::Empty("GeometryCollection"),
                1 => Geometry::LineString(merged.remove(0)),
                _ => Geometry::MultiLineString(merged),
            }))
        }
        "offset_curve" => {
            let line = line_arg(args, "line")?;
            let g = offset::offset_curve(
                &line,
                f64_arg(args, "distance")?,
                int_or(args, "quad_segs", SHAPELY_QUAD_SEGS)?,
            )
            .map_err(geom_err)?;
            ok(geom_json(&g))
        }
        "buffer_line" => {
            let line = line_arg(args, "line")?;
            let g = offset::buffer_line(
                &line,
                f64_arg(args, "distance")?,
                int_or(args, "quad_segs", SHAPELY_QUAD_SEGS)?,
                cap_arg(args)?,
            )
            .map_err(geom_err)?;
            ok(with_ring(g))
        }
        "buffer_point" => {
            let c = [f64_arg(args, "x")?, f64_arg(args, "y")?];
            let g = offset::buffer_point(
                c,
                f64_arg(args, "distance")?,
                int_or(args, "quad_segs", SHAPELY_QUAD_SEGS)?,
            )
            .map_or(Geometry::Empty("Polygon"), Geometry::Polygon);
            ok(with_ring(g))
        }
        "polygon_op" => {
            let a = polys_of(geom_arg(args, "a")?, "args.a")?;
            let b = polys_of(geom_arg(args, "b")?, "args.b")?;
            let op = match super::arg(args, "op")?.as_str() {
                Some("intersection") => Op::Intersection,
                Some("difference") => Op::Difference,
                Some("union") => Op::Union,
                _ => {
                    return Err(bad(
                        "`args.op` must be intersection, difference or union".into()
                    ));
                }
            };
            ok(geom_json(&overlay::overlay_geometry(&a, &b, op)))
        }
        "buffer_shape" => {
            let distance = f64_arg(args, "distance")?;
            let g = match geom_arg(args, "geom")? {
                Geometry::LineString(l) => offset::buffer_line(
                    &l,
                    distance,
                    int_or(args, "quad_segs", SHAPELY_QUAD_SEGS)?,
                    cap_arg(args)?,
                ),
                g @ (Geometry::Polygon(_) | Geometry::MultiPolygon(_)) => {
                    offset::buffer_polygons(&polys_of(g, "args.geom")?, distance)
                }
                _ => return Err(bad("`args.geom` must be a LineString or a polygon".into())),
            }
            .map_err(geom_err)?;
            let valid = valid::is_valid(&polys_of(g.clone(), "the buffer")?);
            ok(json!({"polygon": geom_json(&g), "valid": valid}))
        }
        "is_valid" => {
            let ps = polys_of(geom_arg(args, "polygon")?, "args.polygon")?;
            ok(Value::Bool(valid::is_valid(&ps)))
        }
        "polygon_area" => {
            let ps = polys_of(geom_arg(args, "polygon")?, "args.polygon")?;
            ok(float_json(ps.iter().map(overlay::polygon_area).sum()))
        }
        "contains_xy" => {
            let ps = polys_of(geom_arg(args, "polygon")?, "args.polygon")?;
            let (xs, ys) = (f64_list(args, "xs")?, f64_list(args, "ys")?);
            ok(Value::Array(
                xs.iter()
                    .zip(&ys)
                    .map(|(&x, &y)| Value::Bool(overlay::contains_xy(&ps, [x, y])))
                    .collect(),
            ))
        }
        f => Err(bad(format!("geometry has no parity function `{f}`"))),
    }
}

/// A polygon result with its exterior ring as a plain coordinate list, so that the ring is also
/// checked vertex for vertex (the GEOS circle and line buffers are built exactly).
fn with_ring(g: Geometry) -> Value {
    let ring = match &g {
        Geometry::Polygon(p) => pts_json(&p.exterior),
        _ => json!([]),
    };
    json!({"polygon": geom_json(&g), "ring": ring})
}

/// shapely `a.distance(b)` for points, lines and polygons.
fn distance(a: &Geometry, b: &Geometry) -> Result<f64, EvalError> {
    use Geometry::*;
    Ok(match (a, b) {
        (Point(p), Point(q)) => cd_core::geom::linear::point_line_distance(*p, &[*q]),
        (Point(p), LineString(l)) | (LineString(l), Point(p)) => linear::point_line_distance(*p, l),
        (Point(p), Polygon(poly)) | (Polygon(poly), Point(p)) => {
            overlay::point_polygon_distance(*p, core::slice::from_ref(poly))
        }
        (LineString(l), LineString(m)) => linear::line_line_distance(l, m),
        _ => {
            return Err(bad(
                "distance_geoms takes points, lines and polygons (one polygon at most)".into(),
            ));
        }
    })
}

/// The `synthetic` entry point: `synthetic.py`.
pub(super) fn synthetic(function: &str, args: &Value) -> Result<Value, EvalError> {
    match function {
        "sine_generated_centreline" => {
            let line = synthetic::sine_generated_centreline(
                f64_or(args, "length_m", 6000.0)?,
                f64_or(args, "wavelength_m", 1500.0)?,
                f64_or(args, "amplitude_m", 260.0)?,
                f64_or(args, "step_m", 10.0)?,
            );
            ok(geom_json(&Geometry::LineString(line)))
        }
        "synthetic_river" => {
            let d = RiverParams::default();
            let hill_at = match args.get("hill_at") {
                None | Some(Value::Null) => d.hill_at,
                Some(v) => pt(v, "args.hill_at")?,
            };
            let p = RiverParams {
                length_m: f64_or(args, "length_m", d.length_m)?,
                wavelength_m: f64_or(args, "wavelength_m", d.wavelength_m)?,
                amplitude_m: f64_or(args, "amplitude_m", d.amplitude_m)?,
                width_m: f64_or(args, "width_m", d.width_m)?,
                bed_depth_m: f64_or(args, "bed_depth_m", d.bed_depth_m)?,
                floodplain_above_wl_m: f64_or(
                    args,
                    "floodplain_above_wl_m",
                    d.floodplain_above_wl_m,
                )?,
                bed_slope: f64_or(args, "bed_slope", d.bed_slope)?,
                water_level_at_mouth_m: f64_or(
                    args,
                    "water_level_at_mouth_m",
                    d.water_level_at_mouth_m,
                )?,
                hill_height_m: f64_or(args, "hill_height_m", d.hill_height_m)?,
                hill_at,
                hill_radius_m: f64_or(args, "hill_radius_m", d.hill_radius_m)?,
            };
            let r = synthetic::synthetic_river(p).map_err(geom_err)?;
            let probes = match args.get("probe_xs") {
                None | Some(Value::Null) => Value::Null,
                Some(_) => {
                    let (xs, ys) = (f64_list(args, "probe_xs")?, f64_list(args, "probe_ys")?);
                    Value::Array(
                        xs.iter()
                            .zip(&ys)
                            .map(|(&x, &y)| float_json(r.terrain(x, y)))
                            .collect(),
                    )
                }
            };
            let levels = match args.get("levels_at") {
                None | Some(Value::Null) => vec![],
                Some(_) => f64_list(args, "levels_at")?,
            };
            let p = &r.params;
            ok(json!({
                "river": {
                    "centreline": geom_json(&Geometry::LineString(r.centreline.clone())),
                    "left_bank": geom_json(&r.left_bank),
                    "right_bank": geom_json(&r.right_bank),
                    "polygon": geom_json(&Geometry::Polygon(r.polygon.clone())),
                    "width_m": float_json(p.width_m),
                    "bed_slope": float_json(p.bed_slope),
                    "floodplain_above_wl_m": float_json(p.floodplain_above_wl_m),
                    "water_level_at_mouth_m": float_json(p.water_level_at_mouth_m),
                },
                "terrain_at_probes": probes,
                "water_levels": levels.iter().map(|&s| float_json(r.water_level(s))).collect::<Vec<_>>(),
            }))
        }
        f => Err(bad(format!("synthetic has no parity function `{f}`"))),
    }
}

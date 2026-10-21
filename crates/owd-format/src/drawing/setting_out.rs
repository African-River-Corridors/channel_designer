//! The setting-out table (spec 0004, ticket 4): what a surveyor needs to set out the design line.
//!
//! One row per station at the style's spacing (chainage, easting, northing, bearing, local radius
//! on the chord) and one row per bend (start and end chainage, least radius, deflection), in the
//! design's UTM zone to 0.001 m. The design line is a polyline, not intersection points and arcs
//! (ADR 0007), so there are no IP rows. Every value is the line's own: the point at a chainage, the
//! direction of its segment, the radius of the circle through three points a chord apart (the way the
//! radius check reads it), and the bend units of the bend catalogue. The sheet and the CSV are
//! written from one list of rows, so they hold the same rows.

use super::furniture::{self, Area, Titles, clean};
use super::{DrawError, DrawOptions, Sheet, SheetKind};
use crate::design::Session;
use crate::design::geojson::all_positions;
use crate::design::map::circumradius;
use owd_core::alignment::bends::{bend_units, curvature, resample};
use owd_core::alignment::limits::compute_chainages;
use owd_core::alignment::radius_check::default_check_chord;
use owd_core::geom::{Pt, linear};
use serde_json::{Value, json};

/// The numbers the setting-out table reads, all from the sheet style (`Style::setting_out`).
#[derive(Debug, Clone, PartialEq)]
pub struct SettingOutParams {
    pub sheet_size: String,
    pub landscape: bool,
    pub spacing_m: f64,
    pub row_pitch_mm: f64,
}

/// A station row. `radius_m` is `None` where the chord does not fit on the line, and infinite on a
/// straight.
#[derive(Debug, Clone, PartialEq)]
pub struct StationRow {
    pub chainage_m: f64,
    pub easting_m: f64,
    pub northing_m: f64,
    pub bearing_deg: f64,
    pub radius_m: Option<f64>,
}

/// A bend row.
#[derive(Debug, Clone, PartialEq)]
pub struct BendRow {
    pub start_m: f64,
    pub end_m: f64,
    pub least_radius_m: f64,
    pub deflection_deg: f64,
}

/// The table, from the design line of one option.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub zone: String,
    pub epsg: u64,
    pub chord_m: f64,
    pub stations: Vec<StationRow>,
    pub bends: Vec<BendRow>,
}

/// A radius at or above this is a straight, in metres.
pub const STRAIGHT_M: f64 = 1.0e5;
/// The step the bend catalogue resamples the line at, in metres (the alignment's own default).
const BEND_STEP_M: f64 = 10.0;
/// The smoothing of the curvature that splits bends (points), as the catalogue uses.
const BEND_SMOOTH_N: usize = 5;

fn bearing_deg(a: Pt, b: Pt) -> f64 {
    let d = libm::atan2(b[0] - a[0], b[1] - a[1]).to_degrees();
    if d < 0.0 { d + 360.0 } else { d }
}

/// The compass bearing of the line at chainage `s`: the direction of the segment that starts
/// there (the last segment at the end).
fn bearing_at(line: &[Pt], chain: &[f64], s: f64) -> f64 {
    let i = chain
        .partition_point(|&c| c <= s)
        .saturating_sub(1)
        .min(line.len() - 2);
    bearing_deg(line[i], line[i + 1])
}

/// Read the table's rows from a design line option.
pub fn table(session: &Session, option_id: &str, spacing_m: f64) -> Result<Table, DrawError> {
    let doc = session.document();
    let option = doc["options"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["id"] == option_id))
        .ok_or_else(|| DrawError::Input(format!("there is no option `{option_id}`")))?;
    let line = all_positions(&option["line"]);
    if line.len() < 2 {
        return Err(DrawError::Missing(format!(
            "option `{option_id}` has no design line to set out"
        )));
    }
    let epsg = doc["project"]["crs"]["epsg"].as_u64().ok_or_else(|| {
        DrawError::Missing("the design has no project CRS: the table is in its UTM zone".into())
    })?;
    let zone = doc["project"]["crs"]["name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let chain = compute_chainages(&line);
    let total = *chain.last().unwrap_or(&0.0);
    let r_min = option["checks"]["r_min_m"].as_f64().unwrap_or(f64::NAN);
    let chord = option["checks"]["radius_check_chord_m"]
        .as_f64()
        .filter(|x| x.is_finite() && *x > 0.0)
        .unwrap_or_else(|| default_check_chord(r_min));
    if !chord.is_finite() {
        return Err(DrawError::Missing(
            "the option holds no radius check chord, so there is no local radius to show".into(),
        ));
    }
    let mut at: Vec<f64> = (0..=((total / spacing_m).floor() as usize))
        .map(|i| i as f64 * spacing_m)
        .collect();
    if total - at.last().copied().unwrap_or(0.0) > 1e-6 {
        at.push(total);
    }
    let stations = at
        .iter()
        .map(|&s| {
            let p = linear::interpolate(&line, s).unwrap_or([f64::NAN; 2]);
            let radius_m = (s - chord / 2.0 >= 0.0 && s + chord / 2.0 <= total).then(|| {
                let a = linear::interpolate(&line, s - chord / 2.0).unwrap_or(p);
                let c = linear::interpolate(&line, s + chord / 2.0).unwrap_or(p);
                circumradius(a, p, c)
            });
            StationRow {
                chainage_m: s,
                easting_m: p[0],
                northing_m: p[1],
                bearing_deg: bearing_at(&line, &chain, s),
                radius_m,
            }
        })
        .collect();
    let (s, xy) = resample(&line, BEND_STEP_M);
    let kappa = curvature(&xy);
    let bends = bend_units(&s, &kappa, BEND_SMOOTH_N)
        .into_iter()
        .map(|b| BendRow {
            start_m: b.s0,
            end_m: b.s1,
            least_radius_m: b.r_min_natural_m,
            deflection_deg: b.deflection_rad.to_degrees(),
        })
        .collect();
    Ok(Table {
        zone,
        epsg,
        chord_m: chord,
        stations,
        bends,
    })
}

// ---- how a value is written -----------------------------------------------------------------------

/// A chainage to 0.001 m in the form kilometres plus metres: `1+500.000`.
pub fn chainage_text(m: f64) -> String {
    let mm = (m * 1000.0).round() as i64;
    format!(
        "{}+{:07.3}",
        mm / 1_000_000,
        (mm % 1_000_000) as f64 / 1000.0
    )
}

pub fn coord_text(m: f64) -> String {
    format!("{m:.3}")
}

pub fn bearing_text(deg: f64) -> String {
    format!("{deg:.3}")
}

/// A radius to 0.1 m, `STRAIGHT` where there is none to speak of, `-` where the chord does not fit.
pub fn radius_text(r: Option<f64>) -> String {
    match r {
        None => "-".into(),
        Some(r) if r >= STRAIGHT_M => "STRAIGHT".into(),
        Some(r) => format!("{r:.1}"),
    }
}

fn deflection_text(deg: f64) -> String {
    format!("{deg:.3}")
}

/// The CSV of the table: a header, the station rows, then the bend rows. One file for the design.
pub fn csv(t: &Table) -> String {
    let mut out = String::from(
        "type,chainage_m,easting_m,northing_m,bearing_deg,radius_m,end_chainage_m,least_radius_m,deflection_deg\n",
    );
    for r in &t.stations {
        let radius = match r.radius_m {
            None => String::new(),
            Some(x) if x >= STRAIGHT_M => "straight".into(),
            Some(x) => format!("{x:.1}"),
        };
        out.push_str(&format!(
            "station,{:.3},{},{},{},{radius},,,\n",
            r.chainage_m,
            coord_text(r.easting_m),
            coord_text(r.northing_m),
            bearing_text(r.bearing_deg),
        ));
    }
    for b in &t.bends {
        let least = if b.least_radius_m >= STRAIGHT_M {
            "straight".to_string()
        } else {
            format!("{:.1}", b.least_radius_m)
        };
        out.push_str(&format!(
            "bend,{:.3},,,,,{:.3},{least},{}\n",
            b.start_m,
            b.end_m,
            deflection_text(b.deflection_deg)
        ));
    }
    out
}

// ---- the sheets ---------------------------------------------------------------------------------

const H: f64 = 2.5;
const H_HEAD: f64 = 3.5;
/// Column widths in millimetres, stations then bends.
const STATION_COLS: [(&str, f64); 5] = [
    ("CHAINAGE", 28.0),
    ("EASTING", 30.0),
    ("NORTHING", 30.0),
    ("BEARING", 26.0),
    ("RADIUS", 26.0),
];
const BEND_COLS: [(&str, f64); 4] = [
    ("START CH", 28.0),
    ("END CH", 28.0),
    ("LEAST RADIUS", 30.0),
    ("DEFLECTION", 28.0),
];
/// Paper height of the heading of a table: the title, the unit line, the column titles.
const HEAD_MM: f64 = 20.0;
/// Scales of the two table viewports. The tables are not drawn to a scale; the two differ so that
/// the title block says AS SHOWN, not a scale that means nothing.
const STATION_SCALE: u32 = 1000;
const BEND_SCALE: u32 = 500;

struct Block<'a> {
    id: &'static str,
    prefix: &'static str,
    title: &'a str,
    unit_line: String,
    cols: &'a [(&'static str, f64)],
    rows: Vec<Vec<String>>,
    scale: u32,
    frame: Area,
    pitch: f64,
    /// Model x of the left edge, so that no two windows overlap in the DXF (clause 15.4.9).
    shift_m: f64,
}

/// One table in a viewport whose model is the paper in millimetres, divided by the scale.
fn block(b: &Block) -> Value {
    let k = 1000.0 / f64::from(b.scale);
    let w: f64 = b.cols.iter().map(|c| c.1).sum();
    let h = HEAD_MM + b.pitch * b.rows.len() as f64;
    // Paper (px, py) from the top left of the frame to the model.
    let mx = |px: f64| b.shift_m + px / k;
    let my = |py: f64| (b.frame.h - py) / k;
    let mut e: Vec<Value> = Vec::new();
    let line = |id: String, a: (f64, f64), z: (f64, f64), layer: &str| {
        json!({"type": "line", "layer": layer, "id": id,
               "from_m": [mx(a.0), my(a.1)], "to_m": [mx(z.0), my(z.1)]})
    };
    let text = |id: String, x: f64, y: f64, s: &str, h: f64, align: &str| {
        json!({"type": "text", "layer": "TEXT", "id": id, "anchor_m": [mx(x), my(y)],
               "text": s, "height_mm": h, "align_h": align, "align_v": "baseline"})
    };
    e.push(
        json!({"type": "polyline", "layer": "OUTLINE", "id": format!("{}-outline", b.prefix),
        "closed": true,
        "points_m": [[mx(0.5), my(0.5)], [mx(w), my(0.5)], [mx(w), my(h)], [mx(0.5), my(h)]]}),
    );
    e.push(text(
        format!("{}-title", b.prefix),
        2.0,
        6.0,
        b.title,
        H_HEAD,
        "left",
    ));
    e.push(text(
        format!("{}-units", b.prefix),
        2.0,
        11.0,
        &b.unit_line,
        H,
        "left",
    ));
    e.push(line(
        format!("{}-rule-head", b.prefix),
        (0.5, HEAD_MM - 6.0),
        (w, HEAD_MM - 6.0),
        "OUTLINE",
    ));
    let mut x = 0.0;
    for (i, (name, cw)) in b.cols.iter().enumerate() {
        e.push(text(
            format!("{}-col-{i}", b.prefix),
            x + cw - 2.0,
            HEAD_MM - 2.0,
            name,
            H,
            "right",
        ));
        x += cw;
        if i + 1 < b.cols.len() {
            e.push(line(
                format!("{}-colrule-{i}", b.prefix),
                (x, HEAD_MM - 6.0),
                (x, h),
                "OBJECT",
            ));
        }
    }
    e.push(line(
        format!("{}-rule-cols", b.prefix),
        (0.5, HEAD_MM),
        (w, HEAD_MM),
        "OUTLINE",
    ));
    for (r, row) in b.rows.iter().enumerate() {
        let base = HEAD_MM + b.pitch * (r as f64 + 1.0) - 1.4;
        let mut x = 0.0;
        for (c, cell) in row.iter().enumerate() {
            e.push(text(
                format!("{}-r{r}-c{c}", b.prefix),
                x + b.cols[c].1 - 2.0,
                base,
                cell,
                H,
                "right",
            ));
            x += b.cols[c].1;
        }
        if r + 1 < b.rows.len() {
            e.push(line(
                format!("{}-rule-{r}", b.prefix),
                (0.5, HEAD_MM + b.pitch * (r as f64 + 1.0)),
                (w, HEAD_MM + b.pitch * (r as f64 + 1.0)),
                "OBJECT",
            ));
        }
    }
    json!({
        "id": b.id,
        "frame": {"x_mm": b.frame.x, "y_mm": b.frame.y, "width_mm": b.frame.w, "height_mm": b.frame.h},
        "window": {"xmin_m": b.shift_m, "ymin_m": 0, "xmax_m": mx(b.frame.w), "ymax_m": my(0.0)},
        "scale": b.scale,
        "entities": e,
    })
}

pub(super) fn build(
    session: &Session,
    option_id: &str,
    options: &DrawOptions,
) -> Result<Vec<Sheet>, DrawError> {
    let p = options.style.setting_out();
    let t = table(session, option_id, p.spacing_m)?;
    let doc = session.document();
    let units = format!(
        "COORDINATES IN {} (EPSG:{}), METRES TO 0.001 M. BEARING IN DEGREES FROM NORTH.",
        clean(&t.zone),
        t.epsg
    );
    let notes = vec![
        units.clone(),
        format!(
            "RADIUS: THE CIRCLE THROUGH THE LINE AT THE STATION AND {:.0} M EITHER SIDE (THE CHORD OF THE RADIUS CHECK).",
            t.chord_m / 2.0
        ),
        "THE DESIGN LINE IS A POLYLINE: NO INTERSECTION POINTS. BENDS ARE THE BEND CATALOGUE'S UNITS.".to_string(),
        "THE TABLES ARE NOT DRAWN TO A SCALE. THE SAME ROWS ARE IN THE CSV FILE.".to_string(),
        "SCREENING – NOT FOR CONSTRUCTION.".to_string(),
        format!("SHEET STYLE {} SHA-256 {}", clean(options.style.name()), options.style.hash()),
    ]
    .into_iter()
    .map(|n| n.chars().take(120).collect::<String>())
    .collect::<Vec<_>>();
    let layout = furniture::layout(&p.sheet_size, p.landscape, notes.len()).ok_or_else(|| {
        DrawError::Input(format!(
            "the sheet size `{}` is not one of the standard's",
            p.sheet_size
        ))
    })?;
    let free = layout.free;
    let cap = ((free.h - HEAD_MM) / p.row_pitch_mm).floor() as usize;
    if cap < 1 {
        return Err(DrawError::Input(
            "no row of the setting-out table fits the sheet".into(),
        ));
    }
    let st_w: f64 = STATION_COLS.iter().map(|c| c.1).sum();
    let bd_w: f64 = BEND_COLS.iter().map(|c| c.1).sum();
    if st_w + bd_w + 4.0 > free.w {
        return Err(DrawError::Input(
            "the setting-out tables are wider than the sheet".into(),
        ));
    }
    let station_rows: Vec<Vec<String>> = t
        .stations
        .iter()
        .map(|r| {
            vec![
                chainage_text(r.chainage_m),
                coord_text(r.easting_m),
                coord_text(r.northing_m),
                bearing_text(r.bearing_deg),
                radius_text(r.radius_m),
            ]
        })
        .collect();
    let bend_rows: Vec<Vec<String>> = t
        .bends
        .iter()
        .map(|b| {
            vec![
                chainage_text(b.start_m),
                chainage_text(b.end_m),
                radius_text(Some(b.least_radius_m)),
                deflection_text(b.deflection_deg),
            ]
        })
        .collect();
    let count = station_rows
        .len()
        .div_ceil(cap)
        .max(bend_rows.len().div_ceil(cap))
        .max(1);
    let title = doc["project"]["title"].as_str().unwrap_or("DESIGN");
    let option_name = doc["options"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["id"] == option_id))
        .and_then(|o| o["name"].as_str())
        .unwrap_or(option_id)
        .to_string();
    let date = options
        .date
        .clone()
        .or_else(|| doc["project"]["created"].as_str().map(str::to_string))
        .ok_or_else(|| {
            DrawError::Missing("the design has no created date for the title block".into())
        })?;
    let code = furniture::design_code(&clean(title), SheetKind::SettingOut.code());
    let mut out = Vec::new();
    for si in 0..count {
        let slice = |rows: &[Vec<String>]| -> Vec<Vec<String>> {
            rows.iter().skip(si * cap).take(cap).cloned().collect()
        };
        let part = format!("{} OF {}", si + 1, count);
        let mut viewports = vec![block(&Block {
            id: "so-stations",
            prefix: "so-s",
            title: &format!("SETTING-OUT TABLE: STATIONS ({part})"),
            unit_line: format!("EVERY {} M OF CHAINAGE", p.spacing_m),
            cols: &STATION_COLS,
            rows: slice(&station_rows),
            scale: STATION_SCALE,
            frame: Area {
                x: free.x,
                y: free.y,
                w: st_w + 1.0,
                h: free.h,
            },
            pitch: p.row_pitch_mm,
            shift_m: 0.0,
        })];
        let bends = slice(&bend_rows);
        if !bends.is_empty() {
            viewports.push(block(&Block {
                id: "so-bends",
                prefix: "so-b",
                title: &format!("SETTING-OUT TABLE: BENDS ({part})"),
                unit_line: "CHAINAGE IN METRES, DEFLECTION IN DEGREES".into(),
                cols: &BEND_COLS,
                rows: bends,
                scale: BEND_SCALE,
                frame: Area {
                    x: free.x + st_w + 3.0,
                    y: free.y,
                    w: bd_w + 1.0,
                    h: free.h,
                },
                pitch: p.row_pitch_mm,
                shift_m: 10_000.0,
            }));
        }
        let number = furniture::drawing_number(&code, SheetKind::SettingOut.code(), si + 1);
        let titles = Titles {
            design: clean(title),
            option_name: option_name.clone(),
            sheet_title: "Setting-out table".into(),
            number: number.clone(),
            revision: options.revision.clone(),
            date: date.clone(),
            sheet_number: si + 1,
            sheet_count: count,
        };
        let document = furniture::document(
            &p.sheet_size,
            p.landscape,
            &layout,
            &titles,
            &notes,
            &["OUTLINE", "OBJECT", "TEXT"],
            viewports,
        );
        let extra = if si == 0 {
            vec![(
                format!("{}.csv", number.to_ascii_lowercase()),
                csv(&t).into_bytes(),
            )]
        } else {
            Vec::new()
        };
        out.push(Sheet {
            kind: SheetKind::SettingOut,
            number,
            document,
            extra,
        });
    }
    Ok(out)
}

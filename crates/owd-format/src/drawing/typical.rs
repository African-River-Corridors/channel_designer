//! The typical section (spec 0004, ticket 3): the channel in section with its design basis as
//! dimensions: bottom width, design depth, under-keel clearance and the side slope, the design water
//! level, and the design vessel for scale. One sheet in all, until spec 0007's reaches give one
//! per reach kind.
//!
//! Levels are relative to the channel invert (the water profile gives the real levels, one per
//! chainage). Every value is a result the design holds: the sizing stage's bottom width, depth,
//! draught and beam, and the volumes stage's wet slope. The standard computes each dimension's text
//! from the geometry it points at. The subtraction depth less draught is the definition of the
//! under-keel clearance (the rules give depth as draught plus clearance).

use super::furniture::{self, Area, Titles, clean};
use super::{DrawError, DrawOptions, Sheet, SheetKind};
use crate::design::Session;
use otd::rules::text_width;
use serde_json::{Value, json};

/// The numbers the typical section reads, all from the sheet style (`Style::typical`).
#[derive(Debug, Clone, PartialEq)]
pub struct TypicalParams {
    pub sheet_size: String,
    pub landscape: bool,
    pub scales: Vec<u32>,
    pub detail_scales: Vec<u32>,
    pub min_dimension_mm: f64,
    pub margin_left_mm: f64,
    pub margin_right_mm: f64,
    pub margin_below_mm: f64,
    pub margin_above_mm: f64,
}

/// The results a typical section draws.
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    /// The Rule Set selection, as the sizing results name it.
    pub rule_sets: String,
    pub vessel: String,
    pub source: String,
    pub bottom_width_m: f64,
    pub design_depth_m: f64,
    pub draught_m: f64,
    pub beam_m: f64,
    pub loa_m: f64,
    /// Horizontal run per metre of rise of the wet side slope (cut model's wet batter).
    pub side_slope_h_per_v: f64,
}

impl Facts {
    pub fn ukc_m(&self) -> f64 {
        self.design_depth_m - self.draught_m
    }
}

/// Read the results from the design. A value that is not there stops the sheet and says which.
pub fn facts(session: &Session) -> Result<Facts, DrawError> {
    let doc = session.document();
    let r = &doc["stages"]["sizing"]["results"];
    let get = |v: &Value, what: &str| -> Result<f64, DrawError> {
        v.as_f64().ok_or_else(|| {
            DrawError::Missing(format!(
                "the design holds no {what}: the typical section draws only results the design already has"
            ))
        })
    };
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    Ok(Facts {
        rule_sets: text(&r["rule_sets"]),
        vessel: text(&r["vessel_standard_id"]),
        source: text(&r["source"]),
        bottom_width_m: get(&r["bottom_width_straight_m"], "bottom width (sizing stage)")?,
        design_depth_m: get(&r["design_depth_m"], "design depth (sizing stage)")?,
        draught_m: get(&r["draught_m"], "design draught (sizing stage)")?,
        beam_m: get(&r["beam_m"], "vessel beam (sizing stage)")?,
        loa_m: get(&r["loa_m"], "vessel length (sizing stage)")?,
        side_slope_h_per_v: get(
            &doc["stages"]["volumes"]["settings"]["wet_slope"],
            "wet side slope (volumes stage settings)",
        )?,
    })
}

fn pt(x: f64, y: f64) -> Value {
    json!([x, y])
}

fn text(id: &str, x: f64, y: f64, s: &str, h: f64, align: &str) -> Value {
    json!({
        "type": "text", "layer": "TEXT", "id": id, "anchor_m": pt(x, y), "text": s,
        "height_mm": h, "align_h": align, "align_v": "baseline",
    })
}

/// A viewport with a scale bar is at least this high (clause 7.9.10).
const MIN_SCALE_BAR_FRAME_MM: f64 = 46.0;
const H_LABEL: f64 = 3.5;
const H_SMALL: f64 = 2.5;

fn slope_text(f: &Facts) -> String {
    format!("SIDE SLOPE {:.1}:1 (H:V)", f.side_slope_h_per_v)
}

/// The notes under the drawing: each value with its source.
fn notes(f: &Facts, style: &super::Style) -> Vec<String> {
    let rs = clean(&f.rule_sets);
    let source = clean(&f.source);
    let mut n = vec![
        "LEVELS ARE RELATIVE TO THE CHANNEL INVERT. DIMENSIONS IN METRES.".to_string(),
        format!("BOTTOM WIDTH: RULE SETS {rs}. {source}"),
        format!("DESIGN DEPTH: RULE SETS {rs} (DRAUGHT PLUS UNDER-KEEL CLEARANCE)."),
        format!("UNDER-KEEL CLEARANCE: RULE SETS {rs} (DESIGN DEPTH LESS DRAUGHT)."),
        "SIDE SLOPE: THE VOLUMES STAGE WET BATTER (A SETTING, NOT A RULE OF THE RULE SETS)."
            .to_string(),
        "DESIGN WATER LEVEL: THE WATER PROFILE OF THE DESIGN (ONE LEVEL PER CHAINAGE).".to_string(),
        format!(
            "VESSEL {}: LENGTH {:.1} M, BEAM {:.1} M, DRAUGHT {:.2} M.",
            clean(&f.vessel),
            f.loa_m,
            f.beam_m,
            f.draught_m
        ),
        "SCREENING – NOT FOR CONSTRUCTION.".to_string(),
        format!(
            "SHEET STYLE {} SHA-256 {}",
            clean(style.name()),
            style.hash()
        ),
    ];
    for note in &mut n {
        // A note is one line of at most 120 characters (the schema); it is never shrunk.
        *note = note.chars().take(120).collect();
    }
    n
}

pub(super) fn build(
    session: &Session,
    option_id: &str,
    options: &DrawOptions,
) -> Result<Vec<Sheet>, DrawError> {
    let p = options.style.typical();
    let f = facts(session)?;
    let doc = session.document();
    let (w, d, s) = (f.bottom_width_m, f.design_depth_m, f.side_slope_h_per_v);
    let ukc = f.ukc_m();
    if !(w > 0.0 && d > 0.0 && s > 0.0 && ukc > 0.0 && f.beam_m > 0.0) {
        return Err(DrawError::Missing(
            "the bottom width, depth, slope, clearance and beam must all be positive to draw a section"
                .into(),
        ));
    }
    let xe = w / 2.0 + s * d;
    let notes = notes(&f, &options.style);
    let layout = furniture::layout(&p.sheet_size, p.landscape, notes.len()).ok_or_else(|| {
        DrawError::Input(format!(
            "the sheet size `{}` is not one of the standard's",
            p.sheet_size
        ))
    })?;
    let slope_label = slope_text(&f);
    let label_w = text_width(&slope_label, H_LABEL).unwrap_or(0.0);
    // The largest scale at which the section and its dimensions fit the frame.
    let free = layout.free;
    let mut chosen = None;
    for &scale in &p.scales {
        let k = 1000.0 / f64::from(scale);
        if w.min(d) * k < p.min_dimension_mm {
            continue;
        }
        let left = (p.margin_left_mm + 10.0 + label_w + 4.0) / k;
        let right = p.margin_right_mm / k;
        let (xmin, xmax) = (-xe - left, xe + right);
        let (ymin, ymax) = (-p.margin_below_mm / k, d + p.margin_above_mm / k);
        if (xmax - xmin) * k <= free.w && (ymax - ymin) * k <= free.h {
            chosen = Some((scale, k, [xmin, ymin, xmax, ymax]));
            break;
        }
    }
    let Some((scale, k, win)) = chosen else {
        return Err(DrawError::Input(format!(
            "the typical section does not fit a {} sheet at any scale in the sheet style ({:?})",
            p.sheet_size, p.scales
        )));
    };

    // The section's frame is as high as its window needs, and at least as high as a scale bar needs
    // (clause 7.9.10); the detail takes the rest of the free area.
    let main_h = ((win[3] - win[1]) * k)
        .max(MIN_SCALE_BAR_FRAME_MM)
        .min(free.h);
    let frame = Area { h: main_h, ..free };
    let mut e: Vec<Value> = Vec::new();
    // The channel: the two side slopes and the bottom, thick; the water between them, hatched.
    e.push(json!({
        "type": "polyline", "layer": "OUTLINE", "id": "ts-channel", "closed": false,
        "points_m": [pt(-xe, d), pt(-w / 2.0, 0.0), pt(w / 2.0, 0.0), pt(xe, d)],
    }));
    e.push(json!({
        "type": "hatch", "layer": "HATCH", "id": "ts-water", "pattern": "water",
        "boundary_m": [pt(-xe, d), pt(-w / 2.0, 0.0), pt(w / 2.0, 0.0), pt(xe, d)],
    }));
    // The design water level, a little beyond the banks.
    let stub = 6.0 / k;
    e.push(json!({
        "type": "line", "layer": "WATER", "id": "ts-wl-line", "line_type": "dashed",
        "from_m": pt(-xe - stub, d), "to_m": pt(xe + stub, d),
    }));
    e.push(text(
        "ts-wl-label",
        -xe - stub,
        d + 2.0 / k,
        "DESIGN WATER LEVEL (WL)",
        H_SMALL,
        "left",
    ));
    // The design vessel, at the water level, its keel one under-keel clearance above the invert.
    let b = f.beam_m / 2.0;
    e.push(json!({
        "type": "polyline", "layer": "OBJECT", "id": "ts-vessel", "closed": true,
        "points_m": [pt(-b, ukc), pt(b, ukc), pt(b, d), pt(-b, d)],
    }));
    let vessel_label = clean(&f.vessel);
    if text_width(&vessel_label, H_SMALL).unwrap_or(f64::MAX) + 2.0 < f.beam_m * k
        && f.draught_m * k > H_SMALL + 2.0
    {
        e.push(text(
            "ts-vessel-label",
            0.0,
            ukc + f.draught_m / 2.0 - H_SMALL / 2.0 / k,
            &vessel_label,
            H_SMALL,
            "centre",
        ));
    }
    // The dimensions. A positive offset is to the left of the direction of the dimension: for a
    // horizontal one that is up, for a vertical one the left of the sheet (clauses 12.3.1, 12.4.2).
    e.push(json!({
        "type": "dimension", "layer": "DIMENSIONS", "id": "ts-width", "kind": "linear",
        "orientation": "horizontal", "from": {"point_m": pt(-w / 2.0, 0.0)},
        "to": {"point_m": pt(w / 2.0, 0.0)}, "offset_mm": -10,
    }));
    e.push(json!({
        "type": "dimension", "layer": "DIMENSIONS", "id": "ts-depth", "kind": "linear",
        "orientation": "vertical", "from": {"point_m": pt(w / 2.0, 0.0)},
        "to": {"point_m": pt(xe, d)}, "offset_mm": -(10.0 + s * d * k / 2.0),
    }));
    // The clearance is too small to dimension at this scale: a detail shows it.
    e.push(json!({
        "type": "leader", "layer": "ANNO", "id": "ts-detail-mark",
        "from": {"point_m": pt(b, ukc)},
        "label_at_m": pt(b + 14.0 / k, ukc + 7.0 / k), "text": "SEE DETAIL A",
    }));
    // The side slope, named on the left bank.
    let mid = [-(w / 2.0 + s * d / 2.0), d / 2.0];
    e.push(json!({
        "type": "leader", "layer": "ANNO", "id": "ts-slope",
        "from": {"point_m": pt(mid[0], mid[1])},
        "label_at_m": pt(mid[0] - 12.0 / k, mid[1] - 6.0 / k), "text": slope_label,
    }));

    e.push(border("ts-border", &win, k, frame.w, frame.h));
    let layers = options.style.gallery();
    e.retain(|x| super::gallery::layer_on(&layers, x["layer"].as_str().unwrap_or("")));
    let main = json!({
        "id": "ts",
        "frame": {"x_mm": frame.x, "y_mm": frame.y, "width_mm": frame.w, "height_mm": frame.h},
        "window": {"xmin_m": win[0], "ymin_m": win[1], "xmax_m": win[2], "ymax_m": win[3]},
        "scale": scale,
        "scale_bar": {},
        "entities": e,
    });
    let mut viewports = vec![main];
    viewports.push(detail(&f, &p, free, main_h, &DetailRoom { gap_mm: 2.0 })?);
    let layer_names: Vec<&str> = [
        "OUTLINE",
        "OBJECT",
        "WATER",
        "HATCH",
        "TEXT",
        "DIMENSIONS",
        "ANNO",
    ]
    .into_iter()
    .filter(|l| super::gallery::layer_on(&layers, l))
    .collect();
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
    let code = furniture::design_code(&clean(title), SheetKind::TypicalSection.code());
    let number = furniture::drawing_number(&code, SheetKind::TypicalSection.code(), 1);
    let titles = Titles {
        design: clean(title),
        option_name,
        sheet_title: "Typical section".into(),
        number: number.clone(),
        revision: options.revision.clone(),
        date,
        sheet_number: 1,
        sheet_count: 1,
    };
    let document = furniture::document(
        &p.sheet_size,
        p.landscape,
        &layout,
        &titles,
        &notes,
        &layer_names,
        viewports,
    );
    Ok(vec![Sheet {
        kind: SheetKind::TypicalSection,
        number,
        document,
        extra: Vec::new(),
        warnings: Vec::new(),
    }])
}

struct DetailRoom {
    gap_mm: f64,
}

/// Detail A: the keel and the invert at a scale where the clearance can be dimensioned.
fn detail(
    f: &Facts,
    p: &TypicalParams,
    free: Area,
    main_h: f64,
    room: &DetailRoom,
) -> Result<Value, DrawError> {
    let ukc = f.ukc_m();
    let avail = free.h - main_h - room.gap_mm;
    let frame_at = |h: f64| Area {
        x: free.x,
        y: free.y + main_h + room.gap_mm,
        w: free.w.min(DETAIL_W_MM),
        h,
    };
    // The largest scale at which the dimension is long enough and the detail fits its frame.
    let scale = p
        .detail_scales
        .iter()
        .copied()
        .find(|&sc| {
            let k = 1000.0 / f64::from(sc);
            ukc * k >= p.min_dimension_mm && ukc * k + 2.0 * DETAIL_ROOM_MM <= avail
        })
        .ok_or_else(|| {
            DrawError::Input(format!(
                "the under-keel clearance cannot be dimensioned at any detail scale in the sheet style ({:?}) in a frame {:.0} mm high",
                p.detail_scales, avail
            ))
        })?;
    let k = 1000.0 / f64::from(scale);
    let frame = frame_at(ukc * k + 2.0 * DETAIL_ROOM_MM);
    let (wm, hm) = (frame.w / k, frame.h / k);
    // The detail's model coordinates stand apart from the section's (clause 15.4.9).
    let (x0, y0) = (DETAIL_SHIFT_M - wm / 2.0, ukc / 2.0 - hm / 2.0);
    let top = y0 + hm;
    let half = wm / 2.0 - 12.0 / k;
    let b = (f.beam_m / 2.0).min(half);
    let e: Vec<Value> = vec![
        border(
            "ts-detail-border",
            &[x0, y0, x0 + wm, top],
            k,
            frame.w,
            frame.h,
        ),
        json!({
            "type": "line", "layer": "OUTLINE", "id": "ts-detail-invert",
            "from_m": pt(DETAIL_SHIFT_M - half, 0.0), "to_m": pt(DETAIL_SHIFT_M + half, 0.0),
        }),
        json!({
            "type": "polyline", "layer": "OBJECT", "id": "ts-detail-vessel", "closed": false,
            "points_m": [pt(DETAIL_SHIFT_M - b, ukc + 6.0 / k), pt(DETAIL_SHIFT_M - b, ukc), pt(DETAIL_SHIFT_M + b, ukc), pt(DETAIL_SHIFT_M + b, ukc + 6.0 / k)],
        }),
        json!({
            "type": "dimension", "layer": "DIMENSIONS", "id": "ts-ukc", "kind": "linear",
            "orientation": "vertical", "from": {"point_m": pt(DETAIL_SHIFT_M + b / 2.0, 0.0)},
            "to": {"point_m": pt(DETAIL_SHIFT_M + b / 2.0, ukc)}, "offset_mm": -10,
        }),
        text(
            "ts-detail-title",
            x0 + 2.0 / k,
            top - 6.0 / k,
            &format!("DETAIL A: UNDER-KEEL CLEARANCE, 1:{scale}"),
            H_LABEL,
            "left",
        ),
        text(
            "ts-detail-invert-label",
            DETAIL_SHIFT_M - half,
            -4.0 / k,
            "CHANNEL INVERT",
            H_SMALL,
            "left",
        ),
        text(
            "ts-detail-keel-label",
            DETAIL_SHIFT_M - b,
            ukc + 8.0 / k,
            "VESSEL KEEL AT THE DESIGN DRAUGHT",
            H_SMALL,
            "left",
        ),
    ];
    Ok(json!({
        "id": "ts-detail",
        "frame": {"x_mm": frame.x, "y_mm": frame.y, "width_mm": frame.w, "height_mm": frame.h},
        "window": {"xmin_m": x0, "ymin_m": y0, "xmax_m": x0 + wm, "ymax_m": top},
        "scale": scale,
        "entities": e,
    }))
}

/// The detail is at most this wide on paper, and needs this much height beyond the clearance for
/// its title, labels and the dimension's text.
const DETAIL_W_MM: f64 = 180.0;
const DETAIL_ROOM_MM: f64 = 24.0;

/// The detail's model coordinates start this far along x.
const DETAIL_SHIFT_M: f64 = 10_000.0;

/// A thin border, a hair inside the frame of a viewport whose window is centred in it.
fn border(id: &str, win: &[f64; 4], k: f64, frame_w: f64, frame_h: f64) -> Value {
    let inset = 0.3;
    let (cx, cy) = ((win[0] + win[2]) / 2.0, (win[1] + win[3]) / 2.0);
    let (hx, hy) = (frame_w / 2.0 - inset, frame_h / 2.0 - inset);
    let (hx, hy) = (hx / k, hy / k);
    json!({
        "type": "polyline", "layer": "OBJECT", "id": id, "closed": true,
        "points_m": [pt(cx - hx, cy - hy), pt(cx + hx, cy - hy), pt(cx + hx, cy + hy), pt(cx - hx, cy + hy)],
    })
}

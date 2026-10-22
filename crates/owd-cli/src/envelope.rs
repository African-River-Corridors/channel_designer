//! `owd envelope`: the largest vessel the river allows (spec 0016 T2). A sweep of single-hull
//! sizes on the reference line, kept in the study, as a table, JSON, CSV or a PNG chart. Every
//! output carries the screening notice, the rule set, the draught and the bank caveat.

use crate::commands::{Failure, Result, destination, from_design, input_error, open, say, write};
use owd_core::envelope::{Settings, Size, default_sizes};
use owd_format::design::StageStatus;
use serde_json::{Value, json};

/// The lines every output starts with.
fn header(e: &Value) -> Vec<String> {
    vec![
        e["notice"].as_str().unwrap_or("").to_string(),
        format!(
            "{}; rule set {}; draught {} m.",
            e["label"].as_str().unwrap_or(""),
            e["rule_set"].as_str().unwrap_or("?"),
            e["draught_m"].as_f64().unwrap_or(f64::NAN)
        ),
        format!("Note: {}.", e["bank_caveat"].as_str().unwrap_or("")),
    ]
}

fn size_words(v: &Value) -> String {
    match (v["loa_m"].as_f64(), v["beam_m"].as_f64()) {
        (Some(l), Some(b)) => format!("{l} x {b} m"),
        _ => "none".into(),
    }
}

/// The answer as a table.
fn table(e: &Value) -> String {
    let mut out = header(e).join("\n");
    out.push_str(&format!(
        "\nPlan line: {}. River length {:.0} m.\n",
        e["plan_line"].as_str().unwrap_or("reference"),
        e["length_m"].as_f64().unwrap_or(f64::NAN)
    ));
    if let Some(n) = e["plan_line_note"].as_str() {
        out.push_str(&format!("Note: {n}.\n"));
    }
    out.push_str(&format!(
        "\nLargest size that passes the whole reach: {}\n\n",
        size_words(&e["largest"])
    ));
    out.push_str(&format!(
        "{:<10} {:>8} {:>10} {:>6} {:>9}\n",
        "size", "Rmin m", "bottom m", "passes", "stretches"
    ));
    for s in e["sizes"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{:<10} {:>8.0} {:>10.1} {:>6} {:>9}\n",
            s["label"].as_str().unwrap_or("?"),
            s["r_min_m"].as_f64().unwrap_or(f64::NAN),
            s["bottom_width_straight_m"].as_f64().unwrap_or(f64::NAN),
            if s["passes"] == true { "yes" } else { "no" },
            s["stretch_count"].as_u64().unwrap_or(0)
        ));
    }
    out.push_str("\nLargest size by kilometre:\n");
    for k in e["per_km"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "  {:>7.0} to {:>7.0} m: {}\n",
            k["from_m"].as_f64().unwrap_or(f64::NAN),
            k["to_m"].as_f64().unwrap_or(f64::NAN),
            k["largest"]["label"].as_str().unwrap_or("none")
        ));
    }
    let within: Vec<&str> = e["marks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["within"] == true)
        .filter_map(|m| m["id"].as_str())
        .collect();
    out.push_str(&format!(
        "\nCatalogue classes within the largest size (by length and beam): {}\n",
        if within.is_empty() {
            "none".to_string()
        } else {
            within.join(", ")
        }
    ));
    if let Some(b) = e.get("bed_model") {
        out.push_str(&format!(
            "Bed model: the shallowest assumed depth is {} m, for a draught of {} m (not read by the sweep).\n",
            b["shallowest_depth_m"], b["draught_m"]
        ));
    }
    out
}

/// The per-kilometre strip as CSV: one column per size, `pass` or `blocked`.
fn csv(e: &Value) -> String {
    let mut out: String = header(e).iter().map(|l| format!("# {l}\n")).collect();
    let sizes: Vec<&Value> = e["sizes"].as_array().into_iter().flatten().collect();
    out.push_str("from_m,to_m,largest");
    for s in &sizes {
        out.push_str(&format!(",{}", s["label"].as_str().unwrap_or("?")));
    }
    out.push('\n');
    for k in e["per_km"].as_array().into_iter().flatten() {
        let (a, b) = (
            k["from_m"].as_f64().unwrap_or(f64::NAN),
            k["to_m"].as_f64().unwrap_or(f64::NAN),
        );
        out.push_str(&format!(
            "{a:.0},{b:.0},{}",
            k["largest"]["label"].as_str().unwrap_or("none")
        ));
        for s in &sizes {
            out.push_str(if blocked(s, a, b) {
                ",blocked"
            } else {
                ",pass"
            });
        }
        out.push('\n');
    }
    out
}

/// Whether a size has a stretch in `[a, b)` of the reference line.
fn blocked(s: &Value, a: f64, b: f64) -> bool {
    s["stretches"].as_array().into_iter().flatten().any(|st| {
        st["start_chainage_m"].as_f64().unwrap_or(f64::NAN) < b
            && st["end_chainage_m"].as_f64().unwrap_or(f64::NAN) >= a
    })
}

/// A small RGB raster with an 8 × 8 font, for the chart.
struct Canvas {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

const PAPER: [u8; 3] = [255, 255, 255];
const INK: [u8; 3] = [30, 30, 30];
const PASS: [u8; 3] = [64, 156, 104];
const BLOCKED: [u8; 3] = [196, 64, 52];

impl Canvas {
    fn new(w: usize, h: usize) -> Canvas {
        Canvas {
            w,
            h,
            px: PAPER.repeat(w * h),
        }
    }

    fn rect(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, c: [u8; 3]) {
        for y in y0..y1.min(self.h) {
            for x in x0..x1.min(self.w) {
                let i = (y * self.w + x) * 3;
                self.px[i..i + 3].copy_from_slice(&c);
            }
        }
    }

    fn text(&mut self, x: usize, y: usize, s: &str) {
        use font8x8::UnicodeFonts;
        for (k, ch) in s.chars().enumerate() {
            // The font has no dashes: an en or em dash is drawn as a hyphen.
            let ch = if matches!(ch, '\u{2013}' | '\u{2014}') {
                '-'
            } else {
                ch
            };
            let g = font8x8::BASIC_FONTS
                .get(ch)
                .or_else(|| font8x8::LATIN_FONTS.get(ch))
                .unwrap_or([0; 8]);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..8 {
                    if bits & (1 << col) != 0 {
                        self.rect(
                            x + k * 8 + col,
                            y + row,
                            x + k * 8 + col + 1,
                            y + row + 1,
                            INK,
                        );
                    }
                }
            }
        }
    }

    fn png(&self) -> core::result::Result<Vec<u8>, String> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.w as u32, self.h as u32);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().map_err(|e| e.to_string())?;
            wr.write_image_data(&self.px).map_err(|e| e.to_string())?;
        }
        Ok(out)
    }
}

/// The chart: one row per size, green where it passes and red where a stretch blocks it, along the
/// reference line; under it, the strip of the largest size per kilometre, darker for larger; and
/// the notices.
fn chart(e: &Value) -> core::result::Result<Vec<u8>, String> {
    let sizes: Vec<&Value> = e["sizes"].as_array().into_iter().flatten().collect();
    let km: Vec<&Value> = e["per_km"].as_array().into_iter().flatten().collect();
    let len = e["length_m"].as_f64().unwrap_or(1.0).max(1.0);
    let (left, plot_w, row_h) = (80usize, 900usize, 22usize);
    let notes = header(e);
    let top = 10 + 12;
    let strip_y = top + sizes.len() * row_h + 8;
    let h = strip_y + row_h + 30 + notes.len() * 12 + 10;
    let w = (left + plot_w + 20).max(
        notes
            .iter()
            .map(|n| n.chars().count() * 8 + 20)
            .max()
            .unwrap_or(0),
    );
    let mut c = Canvas::new(w, h);
    c.text(
        10,
        8,
        "Largest vessel the river allows: each size along the reference line",
    );
    let x_of = |m: f64| left + ((m / len).clamp(0.0, 1.0) * plot_w as f64) as usize;
    // Largest size on top.
    for (r, s) in sizes.iter().rev().enumerate() {
        let y = top + r * row_h;
        c.text(8, y + 7, s["label"].as_str().unwrap_or("?"));
        c.rect(left, y + 3, left + plot_w, y + row_h - 3, PASS);
        for st in s["stretches"].as_array().into_iter().flatten() {
            let (a, b) = (
                st["start_chainage_m"].as_f64().unwrap_or(0.0),
                st["end_chainage_m"].as_f64().unwrap_or(0.0),
            );
            let (x0, x1) = (x_of(a), x_of(b).max(x_of(a) + 1));
            c.rect(x0, y + 3, x1, y + row_h - 3, BLOCKED);
        }
    }
    c.text(8, strip_y + 7, "per km");
    let n = sizes.len().max(1);
    for k in &km {
        let (a, b) = (
            k["from_m"].as_f64().unwrap_or(0.0),
            k["to_m"].as_f64().unwrap_or(0.0),
        );
        let label = k["largest"]["label"].as_str();
        let i = label.and_then(|l| sizes.iter().position(|s| s["label"] == l));
        // From pale to dark blue by the size's place in the series; red for none.
        let col = match i {
            Some(i) => {
                let t = (i + 1) as f64 / n as f64;
                let v = |lo: f64, hi: f64| (lo + (hi - lo) * t) as u8;
                [v(198.0, 8.0), v(219.0, 48.0), v(239.0, 107.0)]
            }
            None => BLOCKED,
        };
        c.rect(
            x_of(a),
            strip_y + 3,
            x_of(b).max(x_of(a) + 1),
            strip_y + row_h - 3,
            col,
        );
    }
    let axis = strip_y + row_h + 4;
    c.text(left, axis, "0 km");
    let end = format!("{:.0} km", len / 1000.0);
    c.text(left + plot_w - end.len() * 8, axis, &end);
    for (i, l) in notes.iter().enumerate() {
        c.text(10, axis + 18 + i * 12, l);
    }
    c.png()
}

/// Run the sweep (or, with `show`, print the stored one), write the study back, and print the
/// answer as a table, JSON or CSV; with `png`, also write the chart.
#[allow(clippy::too_many_arguments)]
pub fn envelope(
    doc: &str,
    sizes: Option<&str>,
    draught_m: Option<f64>,
    rule_set: Option<&str>,
    show: bool,
    csv_mode: bool,
    png_path: Option<&str>,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let mut session = open(doc)?;
    let doc_on_stdout = if show {
        false
    } else {
        let settings = Settings {
            sizes: match sizes {
                Some(s) => Size::parse_list(s).map_err(input_error)?,
                None => default_sizes(),
            },
            draught_m: draught_m.unwrap_or(owd_core::envelope::DEFAULT_DRAUGHT_M),
            rule_set: rule_set
                .unwrap_or(owd_core::envelope::DEFAULT_RULE_SET)
                .to_string(),
            clearance_m: 0.0,
        };
        session.run_envelope(settings, true).map_err(from_design)?;
        let dest = destination(doc, output);
        write(&dest, &session.to_text())?;
        dest == "-"
    };
    let status = session.envelope_status();
    let Some(e) = session.envelope_result().cloned() else {
        return Err(input_error(
            "the document holds no sweep yet: run `owd envelope` first",
        ));
    };
    if let Some(p) = png_path {
        let bytes = chart(&e).map_err(|m| Failure {
            message: format!("the chart: {m}"),
            code: 2,
        })?;
        std::fs::write(p, bytes).map_err(|err| input_error(format!("{p}: {err}")))?;
    }
    let mut human = if csv_mode { csv(&e) } else { table(&e) };
    if status == StageStatus::Stale && !csv_mode {
        human.push_str(
            "\nThis sweep is stale: the banks, the reference line or the settings changed since it \
             ran. Run `owd envelope` to replace it.\n",
        );
    }
    say(
        doc_on_stdout,
        json_mode,
        human.trim_end(),
        json!({"ok": true, "status": status.name(), "envelope": e}),
    );
    Ok(())
}

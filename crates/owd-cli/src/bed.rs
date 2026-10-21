//! `owd bed`: give the depth of a river that has no survey (spec 0012, ticket T7).

use crate::commands::{Result, destination, from_design, input_error, open, say, write};
use serde_json::json;

/// The depth table from a file of `chainage_m,depth_m` lines (a first line that is not numbers is
/// a header).
fn read_table(path: &str) -> core::result::Result<Vec<[f64; 2]>, crate::commands::Failure> {
    let text = if path == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
            .map_err(|e| input_error(format!("cannot read stdin: {e}")))?;
        s
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| input_error(format!("cannot read {path}: {e}")))?
    };
    let mut points = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        match (
            f.first().and_then(|x| x.parse::<f64>().ok()),
            f.get(1).and_then(|x| x.parse::<f64>().ok()),
        ) {
            (Some(c), Some(d)) if f.len() == 2 => points.push([c, d]),
            _ if n == 0 => {}
            _ => {
                return Err(input_error(format!(
                    "{path}, line {}: expected `chainage_m,depth_m`",
                    n + 1
                )));
            }
        }
    }
    Ok(points)
}

/// Set (or with `clear`, remove) the bed model of a document.
pub fn bed(
    doc: &str,
    depth: Option<f64>,
    table: Option<&str>,
    season: Option<&str>,
    clear: bool,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let mut session = open(doc)?;
    let human = if clear {
        if depth.is_some() || table.is_some() {
            return Err(input_error("--clear takes no depth"));
        }
        session.clear_bed_model();
        "bed model removed".to_string()
    } else {
        let points = match (depth, table) {
            (Some(_), Some(_)) => {
                return Err(input_error(
                    "give the depth as one --depth or as a --table, not both",
                ));
            }
            (Some(d), None) => vec![[0.0, d]],
            (None, Some(t)) => read_table(t)?,
            (None, None) => {
                return Err(input_error(
                    "give a depth: --depth METRES, or --table FILE of `chainage_m,depth_m` lines",
                ));
            }
        };
        session
            .set_bed_model(&points, season)
            .map_err(from_design)?;
        format!(
            "bed model set: {} depth(s), parabola across the water ({})",
            points.len(),
            owd_format::design::bed::DEPTH_ASSUMED_LABEL
        )
    };
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "bed_model": session.bed_model_value(), "depth_basis": session.depth_note()}),
    );
    Ok(())
}

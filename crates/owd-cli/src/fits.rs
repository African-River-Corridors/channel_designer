//! `owd fits`: the answer to "what vessel fits this reach as it is?" (spec 0012): plan only (T8),
//! or full, with depth and the spec 0007 verdict, when the document has a bed model, a water level
//! and its DEM (T9).

use crate::commands::{
    Failure, Result, destination, from_design, input_error, open_with_dem, read_dem, say, write,
};
use owd_format::design::{DemCheck, StageStatus};
use serde_json::{Value, json};

fn f1(v: &Value) -> String {
    v.as_f64().map_or("-".to_string(), |x| format!("{x:.1}"))
}

/// The answer as a table, one block per Rule Set choice, each stretch under its class.
fn table(results: &Value) -> String {
    let mut out = format!(
        "What fits as it is ({}). River length {:.0} m.\n",
        results["label"].as_str().unwrap_or(""),
        results["length_m"].as_f64().unwrap_or(f64::NAN)
    );
    for rs in results["rule_sets"].as_array().into_iter().flatten() {
        out.push_str(&format!("\n{}\n", rs["rule_set"].as_str().unwrap_or("?")));
        out.push_str(&format!(
            "{:<24} {:>6} {:>6} {:>7} {:>5} {:>10} {:>11} {:>8}\n",
            "class", "LOA m", "beam m", "Rmin m", "fits", "narrow by", "radius by", "stretches"
        ));
        for c in rs["classes"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "{:<24} {:>6} {:>6} {:>7} {:>5} {:>10} {:>11} {:>8}\n",
                c["vessel_standard_id"].as_str().unwrap_or("?"),
                f1(&c["loa_m"]),
                f1(&c["beam_m"]),
                format!("{:.0}", c["r_min_m"].as_f64().unwrap_or(f64::NAN)),
                if c["fits"] == true { "yes" } else { "no" },
                f1(&c["worst_width_deficit_m"]),
                f1(&c["worst_radius_deficit_m"]),
                c["stretch_count"]
            ));
            if let Some(v) = c["verdict"].as_str() {
                out.push_str(&format!("    verdict: {v}\n"));
            }
            for s in c["constraint_stretches"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "    {} from {:.0} m to {:.0} m: needs {}, found {}, short by {}; works: {}, dredge {:.0} m3, dry cut {:.0} m3\n",
                    s["kind"].as_str().unwrap_or("?"),
                    s["start_chainage_m"].as_f64().unwrap_or(f64::NAN),
                    s["end_chainage_m"].as_f64().unwrap_or(f64::NAN),
                    f1(&s["required"]),
                    f1(&s["river"]),
                    f1(&s["deficit"]),
                    s["works"].as_str().unwrap_or("?"),
                    s["dredge_m3"].as_f64().unwrap_or(0.0),
                    s["dry_cut_m3"].as_f64().unwrap_or(0.0),
                ));
            }
            if c["constraint_stretches"].is_array() {
                continue;
            }
            for s in c["stretches"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "    {} from {:.0} m to {:.0} m: needs {} m, found {} m, short by {} m\n",
                    s["kind"].as_str().unwrap_or("?"),
                    s["start_chainage_m"].as_f64().unwrap_or(f64::NAN),
                    s["end_chainage_m"].as_f64().unwrap_or(f64::NAN),
                    f1(&s["required_m"]),
                    f1(&s["found_m"]),
                    f1(&s["deficit_m"]),
                ));
            }
        }
        match rs["largest_fitting"].as_str() {
            Some(c) => out.push_str(&format!("Largest class that fits as it is: {c}\n")),
            None => out.push_str("No class fits as it is.\n"),
        }
    }
    out
}

/// Run the answer (or, with `show`, print the stored one) and write the document back.
pub fn fits(
    doc: &str,
    choices: &[String],
    show: bool,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let (mut session, bytes) = open_with_dem(doc)?;
    if show {
        let status = session.fits_status();
        let Some(results) = session.fits_result().cloned() else {
            return Err(input_error(
                "the document holds no answer yet: run `owd fits` first",
            ));
        };
        let mut human = table(&results);
        if status == StageStatus::Stale {
            human.push_str(
                "\nThis answer is stale: the banks or the centreline changed since it ran. \
                 Run `owd fits` to replace it.\n",
            );
        }
        say(
            false,
            json_mode,
            human.trim_end(),
            json!({"ok": true, "status": status.name(), "fits": results}),
        );
        return Ok(());
    }
    // The DEM only when it is the file whose hash was recorded; with it, a bed model and a water
    // level, the answer is full.
    let dem = match (session.dem_check(bytes.as_deref()), &bytes) {
        (DemCheck::Matches, Some(b)) => Some(read_dem(b, session.dem_path().unwrap_or("DEM"))?),
        _ => None,
    };
    let epsg = session.document()["project"]["crs"]["epsg"]
        .as_u64()
        .map(|c| c as u32);
    let sampler = match &dem {
        Some(d) => Some(owd_format::design::terrain::dem_sampler(d, epsg).map_err(from_design)?),
        None => None,
    };
    session
        .run_fits_with(
            (!choices.is_empty()).then_some(choices),
            sampler
                .as_ref()
                .map(|s| s as &(dyn Fn(&[f64], &[f64]) -> Vec<f64> + Sync)),
        )
        .map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let results = session.fits_result().cloned().ok_or_else(|| Failure {
        message: "the answer did not run".into(),
        code: 2,
    })?;
    say(
        dest == "-",
        json_mode,
        table(&results).trim_end(),
        json!({"ok": true, "status": session.fits_status().name(), "fits": results}),
    );
    Ok(())
}

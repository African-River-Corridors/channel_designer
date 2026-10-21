//! `owd fits`: the plan-only answer to "what vessel fits this reach as it is?" (spec 0012, T8).

use crate::commands::{Failure, Result, destination, from_design, input_error, open, say, write};
use owd_format::design::StageStatus;
use serde_json::{Value, json};

fn f1(v: &Value) -> String {
    v.as_f64().map_or("-".to_string(), |x| format!("{x:.1}"))
}

/// The answer as a table, one block per ruleset, each stretch under its class.
fn table(results: &Value) -> String {
    let mut out = format!(
        "What fits as it is ({}). River length {:.0} m.\n",
        results["label"].as_str().unwrap_or(""),
        results["length_m"].as_f64().unwrap_or(f64::NAN)
    );
    for rs in results["rulesets"].as_array().into_iter().flatten() {
        out.push_str(&format!("\n{}\n", rs["ruleset_id"].as_str().unwrap_or("?")));
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
    rulesets: &[String],
    show: bool,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let mut session = open(doc)?;
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
    session
        .run_fits((!rulesets.is_empty()).then_some(rulesets))
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

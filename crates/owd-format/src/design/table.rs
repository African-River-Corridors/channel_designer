//! The option comparison table and the checks list (spec 0003): what the app's bottom panel shows.
//!
//! Both come from the design, never from the app. A row keeps its numbers whether or not its
//! option passes; failed options sort first, in the order the document lists them within each group.

use super::export::{SCREENING_LABEL, failed_checks, no_feasible_line};
use super::session::Session;
use serde_json::{Value, json};

/// "1 stretch", "3 stretches".
fn stretches(n: usize) -> String {
    format!("{n} stretch{}", if n == 1 { "" } else { "es" })
}

/// Why an option has its verdict, in words, from its constraint stretches and failed checks
/// (#312). The radius stretches read the river's own bends; the R_min check reads the design line.
fn verdict_reason(screening: &Value, failed: &[&str]) -> Option<String> {
    let verdict = screening["verdict"].as_str()?;
    let all = screening["constraint_stretches"].as_array();
    let count = |kind: &str| {
        all.into_iter()
            .flatten()
            .filter(|x| x["kind"] == kind)
            .count()
    };
    let mut parts = Vec::new();
    if verdict == "not-navigable" {
        let n = count("infeasible");
        if n > 0 {
            parts.push(format!(
                "{n} infeasible {}: no line that meets R_min fits in the navigable water",
                if n == 1 { "stretch" } else { "stretches" }
            ));
        }
        if failed.contains(&"R_min") {
            parts.push("the design line is tighter than R_min".to_string());
        }
        if failed.contains(&"no_feasible_line") {
            parts.push("no bend has a feasible line".to_string());
        }
    }
    let works: Vec<String> = ["width", "depth", "radius"]
        .into_iter()
        .filter_map(|k| {
            let n = count(k);
            (n > 0).then(|| format!("{n} {k}"))
        })
        .collect();
    if !works.is_empty() {
        let n: usize = ["width", "depth", "radius"].into_iter().map(count).sum();
        parts.push(format!(
            "{} need works ({})",
            stretches(n),
            works.join(", ")
        ));
    }
    if count("radius") > 0 {
        parts.push("the radius stretches read the river's own bends, not the design line".into());
    }
    if parts.is_empty() {
        parts.push("the river meets every governing limit".into());
    }
    Some(parts.join("; "))
}

impl Session {
    /// The table and the checks list. `dem` is the DEM file the caller found, as for exports: a
    /// changed or missing one makes the volumes `stale`.
    ///
    /// - `rows`: one per option, failed options first: `id`, `name`, `method`, `dredge_m3`,
    ///   `dry_cut_m3`, `total_m3`, `tallest_face_m`, `min_radius_m`, `r_min_m`, `radius_ok`,
    ///   `failed_checks`, and, after the volumes, the screening `verdict` with its
    ///   `verdict_reason` in words. A number the session does not have is `null`.
    /// - `checks`: one per option and check it has (`R_min` always; `fits_in_water`,
    ///   `join_guard` and `no_feasible_line` when they apply), failures first: `option_id`,
    ///   `option`, `check`, `passed` and a `detail` in words.
    pub fn option_table(&self, dem: Option<&[u8]>) -> Value {
        let mut rows = Vec::new();
        let mut checks = Vec::new();
        for o in self.options_list() {
            let id = o["id"].as_str().unwrap_or("");
            let v = self.volumes_row(id);
            let get = |k: &str| v.map_or(Value::Null, |v| v[k].clone());
            let failed = failed_checks(&o);
            let c = &o["checks"];
            let screening = v.map_or(&Value::Null, |v| &v["screening"]);
            rows.push(json!({
                "id": id,
                "name": o["name"],
                "method": o["method"],
                "dredge_m3": get("dredge_m3"),
                "dry_cut_m3": get("excav_m3"),
                "total_m3": get("total_m3"),
                "tallest_face_m": get("max_face_h_m"),
                "min_radius_m": c["measured_min_radius_m"],
                "r_min_m": c["r_min_m"],
                "radius_ok": c["radius_ok"],
                "failed_checks": failed,
                "verdict": screening["verdict"],
                "verdict_reason": verdict_reason(screening, &failed),
            }));
            let passed = !failed.contains(&"R_min");
            let m = |k: &str| {
                c[k].as_f64()
                    .map_or("unknown".to_string(), |x| format!("{x:.0} m"))
            };
            checks.push(json!({
                "option_id": id,
                "option": o["name"],
                "check": "R_min",
                "passed": passed,
                "detail": format!(
                    "minimum radius {} of the design line against the ruleset's {}{}",
                    m("measured_min_radius_m"),
                    m("r_min_m"),
                    c["radius_check_chord_m"].as_f64().map_or(String::new(), |x| format!(", read on a {x:.0} m chord")),
                ),
            }));
            let mut other = |check: &str, passed: bool, detail: String| {
                checks.push(json!({
                    "option_id": id,
                    "option": o["name"],
                    "check": check,
                    "passed": passed,
                    "detail": detail,
                }));
            };
            if c["fits_in_water"].is_object() {
                let ok = c["fits_in_water"]["ok"].as_bool() != Some(false);
                let n = c["fits_in_water"]["stretches"]
                    .as_array()
                    .map_or(0, Vec::len);
                other(
                    "fits_in_water",
                    ok,
                    if ok {
                        "the design line fits in the navigable water".into()
                    } else {
                        format!(
                            "{} where no line that meets R_min fits in the navigable water",
                            stretches(n)
                        )
                    },
                );
            }
            if c["join_guard"].is_object() {
                let ok = c["join_guard"]["ok"].as_bool() != Some(false);
                other(
                    "join_guard",
                    ok,
                    if ok {
                        "every join still holds R_min".into()
                    } else {
                        "a join that held R_min fails after a later fit".into()
                    },
                );
            }
            if no_feasible_line(&o) {
                other(
                    "no_feasible_line",
                    false,
                    "no bend of this water-aware option has a feasible line".into(),
                );
            }
        }
        // Stable: failures first, each group in the document's order.
        rows.sort_by_key(|r| r["failed_checks"].as_array().is_none_or(|a| a.is_empty()));
        checks.sort_by_key(|c| c["passed"].as_bool().unwrap_or(false));
        self.with_depth_basis(json!({
            "screening": SCREENING_LABEL,
            "volumes_status": self.volumes_status_name(dem),
            "rows": rows,
            "checks": checks,
        }))
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use serde_json::json;

    fn session_with(options: Value) -> Session {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/synthetic-river.owd"),
        )
        .unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        doc["options"] = options;
        Session::open(&doc.to_string()).unwrap()
    }

    fn option(id: &str, ok: bool, measured: f64) -> Value {
        json!({"id": id, "name": id.to_uppercase(), "method": "rolling_circle", "settings": {},
            "line": {"type": "LineString", "coordinates": [[0.0, 0.0], [1.0, 1.0]]},
            "checks": {"r_min_m": 330.0, "radius_ok": ok, "measured_min_radius_m": measured,
                       "radius_check_chord_m": 110.0},
            "details": {}})
    }

    #[test]
    fn a_failing_option_sorts_first_and_keeps_its_numbers() {
        let s = session_with(json!([
            option("a", true, 400.0),
            option("b", false, 222.0),
            option("c", true, 350.0)
        ]));
        let t = s.option_table(None);
        let ids: Vec<&str> = t["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["b", "a", "c"],
            "failures first, the rest in the document's order"
        );
        let b = &t["rows"][0];
        assert_eq!(b["failed_checks"], json!(["R_min"]));
        assert_eq!(b["min_radius_m"], 222.0);
        assert_eq!(b["r_min_m"], 330.0);
        assert_eq!(t["screening"], "Screening — not for construction");
        // No volumes yet: the volume columns are null, not zero.
        assert!(b["dredge_m3"].is_null() && b["tallest_face_m"].is_null());
        assert_eq!(t["volumes_status"], "not run");
    }

    #[test]
    fn the_checks_list_has_failures_first_with_the_numbers_in_words() {
        let s = session_with(json!([option("a", true, 400.0), option("b", false, 222.0)]));
        let c = s.option_table(None)["checks"].clone();
        assert_eq!(c[0]["option_id"], "b");
        assert_eq!(c[0]["passed"], false);
        assert_eq!(
            c[0]["detail"],
            "minimum radius 222 m of the design line against the ruleset's 330 m, read on a 110 m chord"
        );
        assert_eq!(c[1]["passed"], true);
    }

    /// The app's own example (#312): its option fails `fits_in_water`, not R_min. The radius
    /// column, the checks and the verdict must agree, and each say what it read.
    #[test]
    fn on_the_example_the_table_the_checks_and_the_verdict_agree() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/web/river-example.owd"),
        )
        .unwrap();
        let t = Session::open(&text).unwrap().option_table(None);
        let row = &t["rows"][0];
        assert_eq!(row["radius_ok"], true, "{row}");
        assert_eq!(row["failed_checks"], json!(["fits_in_water"]));
        let checks: Vec<&Value> = t["checks"].as_array().unwrap().iter().collect();
        let of = |name: &str| *checks.iter().find(|c| c["check"] == name).unwrap();
        // The radius check reads the design line, and passes, as the row's radius column does.
        assert_eq!(of("R_min")["passed"], row["radius_ok"]);
        assert!(
            of("R_min")["detail"]
                .as_str()
                .unwrap()
                .contains("of the design line"),
            "{}",
            of("R_min")
        );
        // The failing check is listed too, with its stretches in words, first.
        let fiw = of("fits_in_water");
        assert_eq!(fiw["passed"], false);
        assert_eq!(checks[0]["check"], "fits_in_water");
        assert!(
            fiw["detail"].as_str().unwrap().contains("3 stretches"),
            "{fiw}"
        );
        // The verdict and its reason: the infeasible stretches, not the radius.
        assert_eq!(row["verdict"], "not-navigable");
        let why = row["verdict_reason"].as_str().unwrap();
        assert!(why.contains("3 infeasible stretches"), "{why}");
        assert!(why.contains("the river's own bends"), "{why}");
    }

    #[test]
    fn a_design_with_no_options_has_an_empty_table() {
        let t = session_with(json!([])).option_table(None);
        assert_eq!(t["rows"], json!([]));
        assert_eq!(t["checks"], json!([]));
    }
}

//! The option comparison table and the checks list (spec 0003): what the app's bottom panel shows.
//!
//! Both come from the design, never from the app. A row keeps its numbers whether or not its
//! option passes; failed options sort first, in the order the document lists them within each group.

use super::export::{SCREENING_LABEL, failed_checks};
use super::session::Session;
use serde_json::{Value, json};

impl Session {
    /// The table and the checks list. `dem` is the DEM file the caller found, as for exports: a
    /// changed or missing one makes the volumes `stale`.
    ///
    /// - `rows`: one per option, failed options first: `id`, `name`, `method`, `dredge_m3`,
    ///   `dry_cut_m3`, `total_m3`, `tallest_face_m`, `min_radius_m`, `r_min_m`, `radius_ok`,
    ///   `failed_checks`. A number the session does not have is `null`.
    /// - `checks`: one per option and check, failures first: `option_id`, `option`, `check`,
    ///   `passed` and a `detail` in words.
    pub fn option_table(&self, dem: Option<&[u8]>) -> Value {
        let mut rows = Vec::new();
        let mut checks = Vec::new();
        for o in self.options_list() {
            let id = o["id"].as_str().unwrap_or("");
            let v = self.volumes_row(id);
            let get = |k: &str| v.map_or(Value::Null, |v| v[k].clone());
            let failed = failed_checks(&o);
            let c = &o["checks"];
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
                    "minimum radius {} against the ruleset's {}{}",
                    m("measured_min_radius_m"),
                    m("r_min_m"),
                    c["radius_check_chord_m"].as_f64().map_or(String::new(), |x| format!(", read on a {x:.0} m chord")),
                ),
            }));
        }
        // Stable: failures first, each group in the document's order.
        rows.sort_by_key(|r| r["failed_checks"].as_array().is_none_or(|a| a.is_empty()));
        checks.sort_by_key(|c| c["passed"].as_bool().unwrap_or(false));
        json!({
            "screening": SCREENING_LABEL,
            "volumes_status": self.volumes_status_name(dem),
            "rows": rows,
            "checks": checks,
        })
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
            "minimum radius 222 m against the ruleset's 330 m, read on a 110 m chord"
        );
        assert_eq!(c[1]["passed"], true);
    }

    #[test]
    fn a_design_with_no_options_has_an_empty_table() {
        let t = session_with(json!([])).option_table(None);
        assert_eq!(t["rows"], json!([]));
        assert_eq!(t["checks"], json!([]));
    }
}

//! Exports (spec 0002): the design line options as GeoJSON, in the project CRS or in WGS84, and the
//! quantity table as CSV. Every export says "Screening — not for construction". An option that
//! fails a check keeps its numbers and says so: a `failed_checks` column in the CSV and a
//! `failed_checks` property in the GeoJSON, never a quiet pass.

use super::crs::{Crs, Utm};
use super::session::{DesignError, Session, Stage, StageStatus};
use serde_json::{Value, json};

/// The label every export carries.
pub const SCREENING_LABEL: &str = "Screening — not for construction";

/// Whether every bend of a water-aware option is infeasible: no feasible line (issue #366).
pub fn no_feasible_line(option: &Value) -> bool {
    option["checks"]["feasible"]["ok"].as_bool() == Some(false)
}

/// The checks an option fails, by name. `R_min` when the minimum radius is under the ruleset's;
/// `fits_in_water` when a water-aware option has an infeasible stretch, and `join_guard` when a
/// join that held R_min failed after a later fit (spec 0005); `no_feasible_line` when every bend
/// of a water-aware option is infeasible (issue #366).
pub fn failed_checks(option: &Value) -> Vec<&'static str> {
    let mut failed = Vec::new();
    if no_feasible_line(option) {
        failed.push("no_feasible_line");
    }
    let c = &option["checks"];
    if c["radius_ok"].as_bool() == Some(false) {
        failed.push("R_min");
    }
    if c["fits_in_water"]["ok"].as_bool() == Some(false) {
        failed.push("fits_in_water");
    }
    if c["join_guard"]["ok"].as_bool() == Some(false) {
        failed.push("join_guard");
    }
    failed
}

/// The note a water-aware option carries in every export: "width only, no depth data".
pub fn water_note(option: &Value) -> Option<&'static str> {
    option["checks"]["fits_in_water"]
        .is_object()
        .then_some(super::alignment::WIDTH_ONLY)
}

/// The notes of an option: "no feasible line", the water note and the depth note, joined; `None`
/// when there is none.
pub fn notes_of(option: &Value, depth: Option<&str>) -> Option<String> {
    let nfl = no_feasible_line(option).then_some(super::alignment::NO_FEASIBLE_LINE);
    let parts: Vec<&str> = [nfl, water_note(option), depth]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// A float in the shortest text that reads back exactly; empty for none.
fn num(v: &Value) -> String {
    v.as_f64().map_or(String::new(), |x| format!("{x}"))
}

impl Session {
    pub(super) fn volumes_row(&self, id: &str) -> Option<&Value> {
        self.doc["stages"]["volumes"]["results"]["options"]
            .as_array()?
            .iter()
            .find(|r| r["id"] == id)
    }

    pub(super) fn volumes_status_name(&self, dem: Option<&[u8]>) -> &'static str {
        match self.stage_status_with_dem(Stage::Volumes, dem) {
            StageStatus::NotRun => "not run",
            StageStatus::Fresh => "fresh",
            StageStatus::Stale => "stale",
        }
    }

    pub(super) fn options_list(&self) -> Vec<Value> {
        self.doc["options"].as_array().cloned().unwrap_or_default()
    }

    /// The quantity table: one comment line with the screening label, a header, then one row per
    /// design line option. Volumes are empty until the volumes stage has run; `volumes_status` says
    /// whether the numbers still hold. `dem` is the DEM file the caller found, if any: a changed or
    /// missing one makes the volumes stale.
    pub fn export_csv(&self, dem: Option<&[u8]>) -> String {
        let mut out = format!("# {SCREENING_LABEL}\n");
        for n in self.notices() {
            out.push_str(&format!("# {}\n", n.replace('\n', " ")));
        }
        out.push_str(
            "option_id,option,method,dredge_m3,dry_cut_m3,total_m3,tallest_face_m,\
             min_radius_m,r_min_m,radius_check_chord_m,radius_ok,failed_checks,volumes_status,notes\n",
        );
        let status = self.volumes_status_name(dem);
        for o in self.options_list() {
            let id = o["id"].as_str().unwrap_or("");
            let v = self.volumes_row(id);
            let get = |k: &str| v.map_or(String::new(), |v| num(&v[k]));
            let failed = failed_checks(&o);
            let cols = [
                csv_field(id),
                csv_field(o["name"].as_str().unwrap_or("")),
                csv_field(o["method"].as_str().unwrap_or("")),
                get("dredge_m3"),
                get("excav_m3"),
                get("total_m3"),
                get("max_face_h_m"),
                num(&o["checks"]["measured_min_radius_m"]),
                num(&o["checks"]["r_min_m"]),
                num(&o["checks"]["radius_check_chord_m"]),
                o["checks"]["radius_ok"]
                    .as_bool()
                    .map_or(String::new(), |b| b.to_string()),
                csv_field(&failed.join(";")),
                status.to_string(),
                csv_field(&notes_of(&o, self.depth_note()).unwrap_or_default()),
            ];
            out.push_str(&cols.join(","));
            out.push('\n');
        }
        out
    }

    /// The design line options as a GeoJSON FeatureCollection: in the project CRS (named in a `crs`
    /// member) or, with `wgs84`, in longitude and latitude. Each feature carries the option's
    /// checks, its failed checks, its quantities when the volumes stage has run, and the label.
    pub fn export_geojson(&self, wgs84: bool, dem: Option<&[u8]>) -> Result<String, DesignError> {
        let epsg = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .map(|c| c as u32);
        let utm: Option<Utm> = epsg
            .and_then(|c| Crs::from_epsg(c).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            });
        if self.options_list().is_empty() {
            return Err(DesignError::Input(
                "there are no design line options to export: run the alignment stage first".into(),
            ));
        }
        let status = self.volumes_status_name(dem);
        let mut features = Vec::new();
        for o in self.options_list() {
            let id = o["id"].as_str().unwrap_or("");
            let mut coords = Vec::new();
            for p in o["line"]["coordinates"].as_array().into_iter().flatten() {
                let (x, y) = (
                    p[0].as_f64().unwrap_or(f64::NAN),
                    p[1].as_f64().unwrap_or(f64::NAN),
                );
                if wgs84 {
                    let u = utm.ok_or_else(|| {
                        DesignError::Input("the channel design has no project CRS".into())
                    })?;
                    let [lon, lat] = u.inverse(x, y).map_err(DesignError::Input)?;
                    coords.push(json!([lon, lat]));
                } else {
                    coords.push(json!([x, y]));
                }
            }
            let failed = failed_checks(&o);
            features.push(json!({
                "type": "Feature",
                "geometry": {"type": "LineString", "coordinates": coords},
                "properties": {
                    "screening": SCREENING_LABEL,
                    "id": id,
                    "name": o["name"],
                    "method": o["method"],
                    "checks": o["checks"],
                    "failed_checks": failed,
                    "notes": notes_of(&o, self.depth_note()),
                    "volumes": self.volumes_row(id),
                    "volumes_status": status,
                },
            }));
        }
        let mut fc = json!({
            "type": "FeatureCollection",
            "screening": SCREENING_LABEL,
            "engine_version": super::session::ENGINE_VERSION,
            "title": self.doc["project"]["title"],
            "features": features,
        });
        let notices = self.notices();
        if !notices.is_empty() {
            fc["notices"] = json!(notices);
        }
        if let Some(note) = self.depth_note() {
            fc["depth_basis"] = json!(note);
        }
        // RFC 7946 is WGS84 only. For the project CRS the legacy `crs` member says which, as the
        // importer reads it back.
        if !wgs84 {
            fc["crs"] = json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:EPSG::{}", epsg.unwrap_or(0))}});
        }
        let mut t = serde_json::to_string_pretty(&fc).expect("a JSON value serialises");
        t.push('\n');
        Ok(t)
    }

    /// The design basis (B) and the verdicts (A) as `#` comment lines, for the head of a CSV.
    fn basis_and_verdict_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let sizing = &self.doc["stages"]["sizing"]["results"];
        for b in sizing["design_basis"].as_array().into_iter().flatten() {
            out.push(format!(
                "design basis: {}{} = {} {} by {} ({}, {}, {})",
                b["quantity"].as_str().unwrap_or(""),
                b["waterway"]
                    .as_str()
                    .map_or(String::new(), |w| format!(" on {w}")),
                num(&b["value"]),
                b["unit"].as_str().unwrap_or(""),
                b["rule"].as_str().unwrap_or(""),
                b["citation"].as_str().unwrap_or(""),
                b["origin"].as_str().unwrap_or(""),
                b["status"].as_str().unwrap_or(""),
            ));
        }
        for o in self.options_list() {
            let id = o["id"].as_str().unwrap_or("");
            if let Some(sc) = self.volumes_row(id).map(|v| &v["screening"]) {
                let blocking = &sc["blocking_not_evaluated"];
                out.push(format!(
                    "verdict: {id}: {} ({} blocking Rules not evaluated{})",
                    sc["verdict"].as_str().unwrap_or(""),
                    blocking["count"],
                    blocking["rules"]
                        .as_array()
                        .filter(|r| !r.is_empty())
                        .map_or(String::new(), |r| format!(
                            ": {}",
                            r.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )),
                ));
            }
        }
        out
    }

    /// The constraint stretches (C) of every option as CSV, one row per stretch. The head carries the
    /// screening label, the unverified-Rules notice, the design basis and the verdicts. Empty rows
    /// below the head until the volumes stage has run.
    pub fn export_stretches_csv(&self, dem: Option<&[u8]>) -> String {
        let mut out = format!(
            "# {SCREENING_LABEL}\n# {}\n",
            super::screening::UNVERIFIED_NOTICE
        );
        for n in self.notices() {
            out.push_str(&format!("# {}\n", n.replace('\n', " ")));
        }
        for l in self.basis_and_verdict_lines() {
            out.push_str(&format!("# {}\n", l.replace('\n', " ")));
        }
        out.push_str(&format!("# volumes: {}\n", self.volumes_status_name(dem)));
        out.push_str(
            "option_id,kind,start_chainage_m,end_chainage_m,start_x,start_y,end_x,end_y,\
             governing_rule,required,river,worst_chainage_m,deficit,works,dredge_m3,dry_cut_m3\n",
        );
        for o in self.options_list() {
            let id = o["id"].as_str().unwrap_or("");
            let Some(v) = self.volumes_row(id) else {
                continue;
            };
            for x in v["screening"]["constraint_stretches"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let cols = [
                    csv_field(id),
                    csv_field(x["kind"].as_str().unwrap_or("")),
                    num(&x["start_chainage_m"]),
                    num(&x["end_chainage_m"]),
                    num(&x["start"][0]),
                    num(&x["start"][1]),
                    num(&x["end"][0]),
                    num(&x["end"][1]),
                    csv_field(x["governing_rule"].as_str().unwrap_or("")),
                    num(&x["required"]),
                    num(&x["river"]),
                    num(&x["worst_chainage_m"]),
                    num(&x["deficit"]),
                    csv_field(x["works"].as_str().unwrap_or("")),
                    num(&x["dredge_m3"]),
                    num(&x["dry_cut_m3"]),
                ];
                out.push_str(&cols.join(","));
                out.push('\n');
            }
        }
        out
    }

    /// The constraint stretches (C) as GeoJSON lines: each the part of its option's design line
    /// between the stretch's ends, in the project CRS or, with `wgs84`, in longitude and latitude.
    pub fn export_stretches_geojson(
        &self,
        wgs84: bool,
        dem: Option<&[u8]>,
    ) -> Result<String, DesignError> {
        let epsg = self.doc["project"]["crs"]["epsg"]
            .as_u64()
            .map(|c| c as u32);
        let utm: Option<Utm> = epsg
            .and_then(|c| Crs::from_epsg(c).ok())
            .and_then(|c| match c {
                Crs::Utm(u) => Some(u),
                Crs::Wgs84 => None,
            });
        let mut features = Vec::new();
        for o in self.options_list() {
            let id = o["id"].as_str().unwrap_or("");
            let Some(v) = self.volumes_row(id) else {
                continue;
            };
            let line = super::geojson::all_positions(&o["line"]);
            for x in v["screening"]["constraint_stretches"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let (a, b) = (
                    x["start_chainage_m"].as_f64().unwrap_or(0.0),
                    x["end_chainage_m"].as_f64().unwrap_or(0.0),
                );
                let mut coords = Vec::new();
                for [px, py] in part_of(&line, a, b) {
                    if wgs84 {
                        let u = utm.ok_or_else(|| {
                            DesignError::Input("the channel design has no project CRS".into())
                        })?;
                        let [lon, lat] = u.inverse(px, py).map_err(DesignError::Input)?;
                        coords.push(json!([lon, lat]));
                    } else {
                        coords.push(json!([px, py]));
                    }
                }
                let mut props = x.clone();
                props["option_id"] = json!(id);
                props["screening"] = json!(SCREENING_LABEL);
                props["rules_notice"] = json!(super::screening::UNVERIFIED_NOTICE);
                features.push(json!({
                    "type": "Feature",
                    "geometry": {"type": "LineString", "coordinates": coords},
                    "properties": props,
                }));
            }
        }
        let mut fc = json!({
            "type": "FeatureCollection",
            "screening": SCREENING_LABEL,
            "rules_notice": super::screening::UNVERIFIED_NOTICE,
            "engine_version": super::session::ENGINE_VERSION,
            "title": self.doc["project"]["title"],
            "volumes_status": self.volumes_status_name(dem),
            "features": features,
        });
        let notices = self.notices();
        if !notices.is_empty() {
            fc["notices"] = json!(notices);
        }
        if !wgs84 {
            fc["crs"] = json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:EPSG::{}", epsg.unwrap_or(0))}});
        }
        let mut t = serde_json::to_string_pretty(&fc).expect("a JSON value serialises");
        t.push('\n');
        Ok(t)
    }
}

/// The part of a line from chainage `a` to `b`: the points at `a` and `b` and every vertex between.
/// A stretch of one station is one point twice.
fn part_of(line: &[[f64; 2]], a: f64, b: f64) -> Vec<[f64; 2]> {
    use owd_core::geom::linear::interpolate;
    let mut out = Vec::new();
    if let Some(p) = interpolate(line, a) {
        out.push(p);
    }
    let mut s = 0.0;
    for w in line.windows(2) {
        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        s += (dx * dx + dy * dy).sqrt();
        if s > a && s < b {
            out.push(w[1]);
        }
    }
    if let Some(p) = interpolate(line, b) {
        out.push(p);
    }
    out
}

//! Exports (spec 0002): the design line options as GeoJSON, in the project CRS or in WGS84, and the
//! quantity table as CSV. Every export says "Screening — not for construction". An option that
//! fails a check keeps its numbers and says so: a `failed_checks` column in the CSV and a
//! `failed_checks` property in the GeoJSON, never a quiet pass.

use super::crs::{Crs, Utm};
use super::session::{DesignError, Session, Stage, StageStatus};
use serde_json::{Value, json};

/// The label every export carries.
pub const SCREENING_LABEL: &str = "Screening — not for construction";

/// The checks an option fails, by name. `R_min` when the minimum radius is under the ruleset's.
pub fn failed_checks(option: &Value) -> Vec<&'static str> {
    let mut failed = Vec::new();
    if option["checks"]["radius_ok"].as_bool() == Some(false) {
        failed.push("R_min");
    }
    failed
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
    fn volumes_row(&self, id: &str) -> Option<&Value> {
        self.doc["stages"]["volumes"]["results"]["options"]
            .as_array()?
            .iter()
            .find(|r| r["id"] == id)
    }

    fn volumes_status_name(&self, dem: Option<&[u8]>) -> &'static str {
        match self.stage_status_with_dem(Stage::Volumes, dem) {
            StageStatus::NotRun => "not run",
            StageStatus::Fresh => "fresh",
            StageStatus::Stale => "stale",
        }
    }

    fn options_list(&self) -> Vec<Value> {
        self.doc["options"].as_array().cloned().unwrap_or_default()
    }

    /// The quantity table: one comment line with the screening label, a header, then one row per
    /// design line option. Volumes are empty until the volumes stage has run; `volumes_status` says
    /// whether the numbers still hold. `dem` is the DEM file the caller found, if any: a changed or
    /// missing one makes the volumes stale.
    pub fn export_csv(&self, dem: Option<&[u8]>) -> String {
        let mut out = format!("# {SCREENING_LABEL}\n");
        out.push_str(
            "option_id,option,method,dredge_m3,dry_cut_m3,total_m3,tallest_face_m,\
             min_radius_m,r_min_m,radius_check_chord_m,radius_ok,failed_checks,volumes_status\n",
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
        // RFC 7946 is WGS84 only. For the project CRS the legacy `crs` member says which, as the
        // importer reads it back.
        if !wgs84 {
            fc["crs"] = json!({"type": "name", "properties": {"name": format!("urn:ogc:def:crs:EPSG::{}", epsg.unwrap_or(0))}});
        }
        let mut t = serde_json::to_string_pretty(&fc).expect("a JSON value serialises");
        t.push('\n');
        Ok(t)
    }
}

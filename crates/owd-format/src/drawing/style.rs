//! The sheet style (spec 0004, ticket 2): a JSON file that says what a sheet shows, with a JSON
//! Schema (`schema/owd-style.schema.json`), a default built into every binary, and a loader.
//!
//! A user's file may hold only the members it changes. It is laid over the default (objects merge,
//! lists are replaced), then CLI-style overrides are set by dotted path, then the result is checked
//! against the schema. The hash of that result goes on every sheet, so the sheet says which rules
//! made it. No threshold of the gallery lives only in code.

use super::gallery::GalleryParams;
use super::long_section::{Exaggeration, LongSectionParams};
use super::setting_out::SettingOutParams;
use super::typical::TypicalParams;
use crate::design::hash::hash_value;
use crate::design::schema::validate;
use serde_json::Value;

/// The schema, as published in the repository.
pub const SCHEMA_TEXT: &str = include_str!("../../../../schema/owd-style.schema.json");
/// The default style, built into every binary.
pub const DEFAULT_TEXT: &str = include_str!("owd-default.style.json");

/// Why a style could not be loaded: every field that breaks the schema, named.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleError(pub String);

impl core::fmt::Display for StyleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for StyleError {}

/// A checked sheet style.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    value: Value,
}

fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (slot, v) => *slot = v.clone(),
    }
}

/// A whole number is written as an integer (`250`, not `250.0`), so the same style has one hash
/// whether a number came from a file or from a flag.
fn whole_numbers(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && n.as_i64().is_none()
                && f.fract() == 0.0
                && f.abs() < 1e15
            {
                *v = Value::from(f as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(whole_numbers),
        Value::Object(o) => o.values_mut().for_each(whole_numbers),
        _ => {}
    }
}

fn check(value: &Value) -> Result<(), StyleError> {
    let schema: Value = serde_json::from_str(SCHEMA_TEXT).expect("the style schema is valid JSON");
    let errors = validate(&schema, value);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(StyleError(format!(
            "the sheet style is not valid: {}",
            errors.join("; ")
        )))
    }
}

impl Style {
    /// The style built into the binary.
    pub fn builtin() -> Style {
        let value: Value =
            serde_json::from_str(DEFAULT_TEXT).expect("owd-default.style.json is valid JSON");
        Style { value }
    }

    /// A user's style: its members laid over the default, then checked.
    pub fn from_overlay(overlay: &Value) -> Result<Style, StyleError> {
        let mut value = Style::builtin().value;
        merge(&mut value, overlay);
        whole_numbers(&mut value);
        check(&value)?;
        Ok(Style { value })
    }

    /// A user's style from text.
    pub fn from_text(text: &str) -> Result<Style, StyleError> {
        let overlay: Value = serde_json::from_str(text)
            .map_err(|e| StyleError(format!("the sheet style is not JSON: {e}")))?;
        Style::from_overlay(&overlay)
    }

    /// Set single values by dotted path (`sections.spacing_m`), then check again. A path that does
    /// not name a member of the style is refused.
    pub fn with_overrides(mut self, overrides: &[(String, Value)]) -> Result<Style, StyleError> {
        for (path, v) in overrides {
            let slot = self
                .value
                .pointer_mut(&format!("/{}", path.replace('.', "/")))
                .ok_or_else(|| {
                    StyleError(format!("`{path}` is not a member of the sheet style"))
                })?;
            *slot = v.clone();
        }
        whole_numbers(&mut self.value);
        check(&self.value)?;
        Ok(self)
    }

    /// The name of the style.
    pub fn name(&self) -> &str {
        self.value["name"].as_str().unwrap_or("")
    }

    /// The content hash (SHA-256, hex) of the style after the overrides. Key order does not matter.
    pub fn hash(&self) -> String {
        hash_value(&self.value)
    }

    /// The style as JSON.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// What the setting-out table reads.
    pub fn setting_out(&self) -> SettingOutParams {
        let v = &self.value;
        SettingOutParams {
            sheet_size: v["sheets"]["so"]["size"]
                .as_str()
                .unwrap_or("A3")
                .to_string(),
            landscape: v["sheets"]["so"]["orientation"] != "portrait",
            spacing_m: v["setting_out"]["spacing_m"].as_f64().unwrap_or(100.0),
            row_pitch_mm: v["setting_out"]["row_pitch_mm"].as_f64().unwrap_or(4.5),
        }
    }

    /// What the plan sheets read.
    pub fn plan(&self) -> super::plan::PlanParams {
        let v = &self.value;
        let pl = &v["plan"];
        let r = &pl["relief"];
        let num = |x: &Value| x.as_f64().unwrap_or(0.0);
        let rules = pl["label_rules"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| match r["rule"].as_str() {
                        Some("bend-tighter-than") => {
                            Some(super::plan::LabelRule::BendTighterThan {
                                k: r["k"].as_f64().unwrap_or(1.0),
                            })
                        }
                        Some("infeasible-stretch") => {
                            Some(super::plan::LabelRule::InfeasibleStretch)
                        }
                        Some("constraint-stretch") => {
                            Some(super::plan::LabelRule::ConstraintStretch {
                                fields: r["fields"]
                                    .as_array()
                                    .map(|f| {
                                        f.iter()
                                            .filter_map(Value::as_str)
                                            .map(str::to_string)
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            })
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        super::plan::PlanParams {
            sheet_size: v["sheets"]["pl"]["size"]
                .as_str()
                .unwrap_or("A1")
                .to_string(),
            landscape: v["sheets"]["pl"]["orientation"] != "portrait",
            strip_scale: pl["strip_scale"].as_u64().unwrap_or(5000) as u32,
            overlap_m: num(&pl["overlap_m"]),
            margin_m: num(&pl["margin_m"]),
            tick_mm: num(&pl["tick_mm"]),
            tick_label_every: pl["tick_label_every"].as_u64().unwrap_or(5).max(1) as usize,
            light: otd::raster::Light {
                azimuth_deg: num(&r["azimuth_deg"]),
                altitude_deg: num(&r["altitude_deg"]),
                z_factor: num(&r["z_factor"]),
            },
            pixel_mm: num(&r["pixel_mm"]),
            quality: r["quality"].as_u64().unwrap_or(85) as u8,
            rules,
            grid: v["layers"]["GRID"] == true,
            water: v["layers"]["WATER"] == true,
        }
    }

    /// What the typical section reads.
    pub fn typical(&self) -> TypicalParams {
        let v = &self.value;
        let t = &v["typical"];
        let m = &t["margins_mm"];
        let num = |x: &Value| x.as_f64().unwrap_or(0.0);
        TypicalParams {
            sheet_size: v["sheets"]["ts"]["size"]
                .as_str()
                .unwrap_or("A3")
                .to_string(),
            landscape: v["sheets"]["ts"]["orientation"] != "portrait",
            scales: t["scales"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_u64)
                        .map(|n| n as u32)
                        .collect()
                })
                .unwrap_or_default(),
            detail_scales: t["detail_scales"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_u64)
                        .map(|n| n as u32)
                        .collect()
                })
                .unwrap_or_default(),
            min_dimension_mm: num(&t["min_dimension_mm"]),
            margin_left_mm: num(&m["left"]),
            margin_right_mm: num(&m["right"]),
            margin_below_mm: num(&m["below"]),
            margin_above_mm: num(&m["above"]),
        }
    }

    /// What the long section reads. The schema holds `vertical_exaggeration` to `"auto"` or a whole
    /// number of 2 or more, so nothing else reaches here.
    pub fn long_section(&self) -> LongSectionParams {
        let v = &self.value;
        let l = &v["long_section"];
        let bands = &l["bands_mm"];
        let num = |x: &Value| x.as_f64().unwrap_or(0.0);
        LongSectionParams {
            sheet_size: v["sheets"]["ls"]["size"]
                .as_str()
                .unwrap_or("A1")
                .to_string(),
            landscape: v["sheets"]["ls"]["orientation"] != "portrait",
            scales: l["scales"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_u64)
                        .map(|n| n as u32)
                        .collect()
                })
                .unwrap_or_default(),
            exaggeration: match l["vertical_exaggeration"].as_u64() {
                Some(e) => Exaggeration::Factor(u32::try_from(e).unwrap_or(u32::MAX)),
                None => Exaggeration::Auto,
            },
            auto_min_fill: num(&l["auto_min_fill"]),
            below_invert_m: num(&l["below_invert_m"]),
            above_ground_m: num(&l["above_ground_m"]),
            band_left_mm: num(&bands["left"]),
            band_right_mm: num(&bands["right"]),
            band_top_mm: num(&bands["top"]),
            band_bottom_mm: num(&bands["bottom"]),
            chainage_pitch_mm: num(&l["chainage_pitch_mm"]),
            level_pitch_mm: num(&l["level_pitch_mm"]),
            grid: v["layers"]["GRID"] == true,
            water: v["layers"]["WATER"] == true,
        }
    }

    /// What the cross-section gallery reads.
    pub fn gallery(&self) -> GalleryParams {
        let v = &self.value;
        let num = |x: &Value| x.as_f64().unwrap_or(0.0);
        let g = &v["gallery"];
        let bands = &g["bands_mm"];
        GalleryParams {
            sheet_size: v["sheets"]["xs"]["size"]
                .as_str()
                .unwrap_or("A3")
                .to_string(),
            landscape: v["sheets"]["xs"]["orientation"] != "portrait",
            section_spacing_m: num(&v["sections"]["spacing_m"]),
            bend_sections: v["sections"]["extra"]
                .as_array()
                .is_some_and(|a| a.iter().any(|r| r["rule"] == "bend-middle")),
            worst_sections: v["sections"]["extra"]
                .as_array()
                .is_some_and(|a| a.iter().any(|r| r["rule"] == "constraint-worst")),
            scales: g["scales"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_u64)
                        .map(|n| n as u32)
                        .collect()
                })
                .unwrap_or_default(),
            min_columns: g["min_columns"].as_u64().unwrap_or(1) as usize,
            gap_mm: num(&g["gap_mm"]),
            margin_m: num(&g["margin_m"]),
            min_half_m: num(&g["min_half_m"]),
            below_invert_m: num(&g["below_invert_m"]),
            above_ground_m: num(&g["above_ground_m"]),
            band_left_mm: num(&bands["left"]),
            band_right_mm: num(&bands["right"]),
            band_top_mm: num(&bands["top"]),
            band_bottom_mm: num(&bands["bottom"]),
            label_pitch_mm: num(&g["label_pitch_mm"]),
            hatch_min_cut_m: num(&g["hatch_min_cut_m"]),
            grid: v["layers"]["GRID"] == true,
            hatch: v["layers"]["HATCH"] == true,
            water: v["layers"]["WATER"] == true,
        }
    }
}

//! The sheet style (spec 0004, ticket 2): a JSON file that says what a sheet shows, with a JSON
//! Schema (`schema/owd-style.schema.json`), a default built into every binary, and a loader.
//!
//! A user's file may hold only the members it changes. It is laid over the default (objects merge,
//! lists are replaced), then CLI-style overrides are set by dotted path, then the result is checked
//! against the schema. The hash of that result goes on every sheet, so the sheet says which rules
//! made it. No threshold of the gallery lives only in code.

use super::gallery::GalleryParams;
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

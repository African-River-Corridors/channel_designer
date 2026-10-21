//! The Rule Set selection of a channel design (spec 0007, ADR 0008): which sets size the channel, with
//! which inputs, and whether the user chose each (`typed`) or took the tool's choice (`default`).
//!
//! The sizing settings store each selected set's id, version and content hash. The engine's library
//! (the built-in sets, plus any set file the caller loads at run time) gives the sets themselves. A
//! stored hash that the library does not hold is refused, never silently re-evaluated.

use super::session::{DesignError, Session};
use arc_rules::load::Library;
use owd_core::rules::engine::{self, ChannelDomain, Selection};
use owd_core::rules::rulesets::{ChannelShape, channel_shape_for};
use serde_json::{Value, json};

/// The set a new design selects, with its inputs, all marked `default`.
pub const DEFAULT_SETS: &[&str] = &["pianc-wg141"];
/// The inputs of the default selection.
pub const DEFAULT_INPUTS: &[(&str, &str)] = &[(engine::SE_QUALITY, "C")];

/// The inputs a user types for a set. The vessel gives `formation` and `vessel_class`.
const SET_INPUTS: &[&str] = &[engine::SE_QUALITY, engine::RANGE_END];

/// The selection an old ruleset id maps to (spec 0007, "Rule Sets at 0.3.0"): a set and one input.
pub fn old_ruleset(
    ruleset_id: &str,
) -> Result<(&'static str, &'static str, &'static str), DesignError> {
    Ok(match ruleset_id {
        "PIANC-A" => ("pianc-wg141", engine::SE_QUALITY, "A"),
        "PIANC-B" => ("pianc-wg141", engine::SE_QUALITY, "B"),
        "PIANC-C" => ("pianc-wg141", engine::SE_QUALITY, "C"),
        "CN" => ("gb50139", engine::RANGE_END, "permissive"),
        "CN-conservative" => ("gb50139", engine::RANGE_END, "strict"),
        "CN-reduced" => {
            return Err(DesignError::Input(
                "CN-reduced is now gb50139 with range_end strict on a canal reach with the \
                 reduced-radius claim; reaches are not in the document yet"
                    .into(),
            ));
        }
        other => {
            return Err(DesignError::Input(format!(
                "unknown ruleset `{other}`; choose a Rule Set with --rule-set (see `owd rules`)"
            )));
        }
    })
}

fn set_entry(library: &Library, id: &str, source: &str) -> Result<Value, DesignError> {
    let set = library
        .lookup(id)
        .ok_or_else(|| DesignError::Input(format!("unknown Rule Set `{id}`; see `owd rules`")))?;
    Ok(json!({"id": set.id, "version": set.version, "hash": set.hash, "source": source}))
}

/// The sizing settings of a new design: the default selection and no vessel yet.
pub(super) fn default_settings() -> Value {
    let lib = engine::built_in();
    let mut sets: Vec<Value> = DEFAULT_SETS
        .iter()
        .map(|id| set_entry(lib, id, "default").expect("a built-in set"))
        .collect();
    sets.push(set_entry(lib, engine::OWD_SET, "default").expect("a built-in set"));
    json!({
        "rule_sets": sets,
        "set_inputs": DEFAULT_INPUTS
            .iter()
            .map(|(k, v)| json!({"name": k, "value": v, "source": "default"}))
            .collect::<Vec<_>>(),
        "vessel_standard_id": null,
        "draught_m": null,
    })
}

impl Session {
    /// The Rule Sets this session can select: the built-in ones and any loaded at run time.
    pub fn rule_set_library(&self) -> &Library {
        &self.library
    }

    /// Load a Rule Set file at run time, through the same loader as the built-in sets. A file with
    /// the id and version of a loaded set but other content is refused. Returns `id@version`.
    pub fn load_rule_set(&mut self, text: &str) -> Result<String, DesignError> {
        self.library
            .add_text(text, &ChannelDomain)
            .map_err(|e| DesignError::Input(format!("the Rule Set file is refused: {e}")))
    }

    /// Choose the Rule Sets and set inputs, marked `typed`. With no set given the selection keeps
    /// its sets; each input given replaces one of that name. `owd` is always selected. A
    /// `pianc-wg141` with no quality takes quality C, marked `default`. Nothing runs: the sizing
    /// stage goes stale.
    pub fn set_rule_sets(
        &mut self,
        ids: &[&str],
        inputs: &[(&str, &str)],
    ) -> Result<(), DesignError> {
        let mut settings = self.sizing_settings();
        if !ids.is_empty() {
            let mut sets = Vec::new();
            for id in ids {
                let entry = set_entry(&self.library, id, "typed")?;
                if !sets.iter().any(|s: &Value| s["id"] == entry["id"]) {
                    sets.push(entry);
                }
            }
            if !sets.iter().any(|s| s["id"] == engine::OWD_SET) {
                sets.push(set_entry(&self.library, engine::OWD_SET, "default")?);
            }
            settings["rule_sets"] = json!(sets);
            // Inputs of sets no longer selected go; a default input is chosen again below.
            let keep: Vec<Value> = settings["set_inputs"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|i| {
                    i["source"] == "typed" && self.input_is_read(&settings, i["name"].as_str())
                })
                .collect();
            settings["set_inputs"] = json!(keep);
        }
        let mut list = settings["set_inputs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (name, value) in inputs {
            if !SET_INPUTS.contains(name) {
                return Err(DesignError::Input(format!(
                    "unknown set input `{name}`; the set inputs are {}",
                    SET_INPUTS.join(", ")
                )));
            }
            let def = engine::INPUTS
                .iter()
                .find(|(n, _)| n == name)
                .expect("listed");
            if !def.1.contains(value) {
                return Err(DesignError::Input(format!(
                    "set input `{name}` is `{value}`; its values are {}",
                    def.1.join(", ")
                )));
            }
            list.retain(|i| i["name"] != *name);
            list.push(json!({"name": name, "value": value, "source": "typed"}));
        }
        settings["set_inputs"] = json!(list);
        self.add_default_inputs(&mut settings);
        self.check_inputs(&settings)?;
        self.doc["stages"]["sizing"]["settings"] = settings;
        Ok(())
    }

    /// Choose the vessel class and draught. Nothing runs.
    pub fn set_vessel(
        &mut self,
        vessel_standard_id: &str,
        draught_m: Option<f64>,
    ) -> Result<(), DesignError> {
        if let Some(d) = draught_m
            && !(d.is_finite() && d > 0.0)
        {
            return Err(DesignError::Input(
                "the draught must be a positive number of metres".into(),
            ));
        }
        owd_core::rules::vessel_catalogue()
            .iter()
            .find(|(id, _)| id == vessel_standard_id)
            .ok_or_else(|| {
                DesignError::Input(format!("unknown vessel standard `{vessel_standard_id}`"))
            })?;
        let mut settings = self.sizing_settings();
        settings["vessel_standard_id"] = json!(vessel_standard_id);
        settings["draught_m"] = json!(draught_m);
        // The shape must size before the choice is kept, so a set that cannot size it is refused here.
        self.shape_for_settings(&settings)?;
        self.doc["stages"]["sizing"]["settings"] = settings;
        Ok(())
    }

    /// The sizing settings, or the default ones if the document has none.
    pub(super) fn sizing_settings(&self) -> Value {
        let s = &self.doc["stages"]["sizing"]["settings"];
        if s.is_object() {
            s.clone()
        } else {
            default_settings()
        }
    }

    /// Whether a selected set has a Rule that applies on input `name`.
    fn input_is_read(&self, settings: &Value, name: Option<&str>) -> bool {
        let Some(name) = name else { return false };
        selected(settings).iter().any(|(id, version)| {
            self.library.find(id, version).is_some_and(|set| {
                set.rules
                    .iter()
                    .any(|r| r.applies_when.conditions.iter().any(|(k, _)| k == name))
            })
        })
    }

    /// A set input a selected set reads but the user did not type takes its default, marked
    /// `default`; a default input no selected set reads goes.
    fn add_default_inputs(&self, settings: &mut Value) {
        let snapshot = settings.clone();
        let list = settings["set_inputs"].as_array_mut().expect("a list");
        for (name, value) in DEFAULT_INPUTS {
            if !list.iter().any(|i| i["name"] == *name) {
                list.push(json!({"name": name, "value": value, "source": "default"}));
            }
        }
        list.retain(|i| {
            i["source"] == "typed" || self.input_is_read(&snapshot, i["name"].as_str())
        });
    }

    /// Each set input a selected set's Rules apply on has a value.
    fn check_inputs(&self, settings: &Value) -> Result<(), DesignError> {
        for name in SET_INPUTS {
            if self.input_is_read(settings, Some(name))
                && !settings["set_inputs"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|i| i["name"] == *name))
            {
                let def = engine::INPUTS
                    .iter()
                    .find(|(n, _)| n == name)
                    .expect("listed");
                return Err(DesignError::Input(format!(
                    "a selected Rule Set applies on `{name}`: choose it with --set-input {name}=<{}>",
                    def.1.join("|")
                )));
            }
        }
        Ok(())
    }

    /// The engine selection of the sizing settings. A set the library does not hold, or holds with
    /// another content hash, is refused.
    pub fn selection(&self) -> Result<Selection, DesignError> {
        selection_of(&self.library, &self.sizing_settings())
    }

    /// The channel shape of the sizing settings.
    pub(super) fn shape_for_settings(&self, settings: &Value) -> Result<ChannelShape, DesignError> {
        let Some(vessel) = settings["vessel_standard_id"].as_str() else {
            return Err(DesignError::Input(
                "the sizing stage has no vessel class: choose one first".into(),
            ));
        };
        let selection = selection_of(&self.library, settings)?;
        channel_shape_for(&selection, vessel, settings["draught_m"].as_f64())
            .map_err(|e| DesignError::Input(e.raw_message().to_string()))
    }

    /// The channel shape the sizing stage chose.
    pub(super) fn sized_shape(&self) -> Result<ChannelShape, DesignError> {
        self.shape_for_settings(&self.sizing_settings())
    }

    /// The content hash the library holds for each selected set, or null: part of every stage's
    /// dependency hash, so a changed set makes the stages that used it stale.
    pub(super) fn selected_set_hashes(&self) -> Value {
        let settings = self.sizing_settings();
        json!(
            selected(&settings)
                .iter()
                .map(|(id, version)| json!({
                    "set": format!("{id}@{version}"),
                    "hash": self.library.find(id, version).map(|s| s.hash.clone()),
                }))
                .collect::<Vec<_>>()
        )
    }

    /// The selection as words, for the results and the drawings: each set but `owd`, with the set
    /// inputs, such as `pianc-wg141@0.3.0 (se_quality B)`.
    pub(super) fn selection_label(settings: &Value) -> String {
        let sets: Vec<String> = selected(settings)
            .into_iter()
            .filter(|(id, _)| id != engine::OWD_SET)
            .map(|(id, v)| format!("{id}@{v}"))
            .collect();
        let inputs: Vec<String> = settings["set_inputs"]
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|i| {
                        format!(
                            "{} {}",
                            i["name"].as_str().unwrap_or(""),
                            i["value"].as_str().unwrap_or("")
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        if inputs.is_empty() {
            sets.join(" + ")
        } else {
            format!("{} ({})", sets.join(" + "), inputs.join(", "))
        }
    }
}

/// The `(id, version)` of each stored set, in order.
fn selected(settings: &Value) -> Vec<(String, String)> {
    settings["rule_sets"]
        .as_array()
        .map(|l| {
            l.iter()
                .map(|s| {
                    (
                        s["id"].as_str().unwrap_or("").to_string(),
                        s["version"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn selection_of(library: &Library, settings: &Value) -> Result<Selection, DesignError> {
    let stored = settings["rule_sets"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for s in &stored {
        let (id, version) = (
            s["id"].as_str().unwrap_or(""),
            s["version"].as_str().unwrap_or(""),
        );
        let set = library.find(id, version).ok_or_else(|| {
            DesignError::Input(format!(
                "the design selects Rule Set {id}@{version}, which is not loaded: load its file \
                 with --rule-set-file"
            ))
        })?;
        if s["hash"].as_str() != Some(set.hash.as_str()) {
            return Err(DesignError::Input(format!(
                "the design selects Rule Set {id}@{version} with content hash {}, but the loaded \
                 set has {}: the rules changed. Choose the set again (owd size --rule-set {id}) \
                 and rerun",
                s["hash"].as_str().unwrap_or("?"),
                set.hash
            )));
        }
    }
    let chosen = selected(settings);
    let refs: Vec<(&str, &str)> = chosen
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let inputs = settings["set_inputs"]
        .as_array()
        .map(|l| {
            l.iter()
                .map(|i| {
                    (
                        i["name"].as_str().unwrap_or("").to_string(),
                        i["value"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Selection::from_library(library, &refs, inputs)
        .map_err(|e| DesignError::Input(e.raw_message().to_string()))
}

//! Channel sizing rules. `engine` holds the channel rules on the shared rules engine (`arc-rules`,
//! spec 0007): the Rule Sets are data, the strictest Rule governs. `rulesets` sizes a channel from
//! a selection of Rule Sets (`channel_shape_for`) and keeps the width methods (`size_channel`);
//! `screening` finds the constraint stretches and the design basis. `methods` (the width method
//! registry), `design` (per-chainage sizing along a centreline), `simple` (one-call sizing) and
//! `width_profile` mirror the `rules/` package of the frozen Python.
//!
//! Vessel classes and the `Check` type come from `ovcd-core`, pinned to a tag (ADR 0001).
//!
//! Width methods stay pluggable from outside the crate: a [`methods::WidthMethod`] holds plain
//! closures and lives in a registry value that the sizing functions take by reference. Python keeps
//! the registry in a module global; here [`methods::global_methods`] is the same thing for a caller
//! that wants one process-wide registry. A Rule Set is added as a file, loaded at run time.
//! The width methods are design assumptions, not verified rules.

/// Vessel classes (from `ovcd-core`, pinned to a tag): key and class.
pub use ovcd_core::vessel_standards::{Catalogue, VesselStandard};

/// The 12 classes OWD checks by default (`owd fits`), keyed by catalogue id, in their old order
/// (spec 0015 T0). [`full_catalogue`] has every class a channel can be sized for.
pub fn vessel_catalogue() -> Catalogue {
    classes::default_class_ids()
        .into_iter()
        .filter_map(|id| classes::class(id).ok().map(|v| (id.to_string(), v)))
        .collect()
}

/// Every class in the shared catalogue a channel can be sized for, keyed by catalogue id.
pub fn full_catalogue() -> Catalogue {
    classes::pickable_classes()
        .into_iter()
        .map(|v| (v.standard_id.clone(), v))
        .collect()
}

pub mod classes;
pub mod custom;
pub mod design;
pub mod engine;
pub mod methods;
pub mod rulesets;
pub mod screening;
pub mod simple;
pub mod width_profile;

/// The check type, from `ovcd-core` (re-exported so callers need no second dependency).
pub use ovcd_core::checks::Check;

/// A refusal the Python raises. `Value` is a `ValueError`, `Key` a `KeyError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    Value(String),
    Key(String),
}

impl RuleError {
    /// The Python exception class name.
    pub fn type_name(&self) -> &'static str {
        match self {
            RuleError::Value(_) => "ValueError",
            RuleError::Key(_) => "KeyError",
        }
    }

    /// The text as given to the exception, without Python's `str()` quoting of a `KeyError`.
    pub fn raw_message(&self) -> &str {
        match self {
            RuleError::Value(m) | RuleError::Key(m) => m,
        }
    }

    /// `str(exc)`: a `KeyError` shows the `repr` of its message, a `ValueError` the message.
    pub fn python_message(&self) -> String {
        match self {
            RuleError::Value(m) => m.clone(),
            RuleError::Key(m) => crate::pyrepr::repr_str(m),
        }
    }
}

impl core::fmt::Display for RuleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.type_name(), self.python_message())
    }
}

impl std::error::Error for RuleError {}

pub(crate) fn value_err<T>(message: String) -> Result<T, RuleError> {
    Err(RuleError::Value(message))
}

/// Python's `repr()` of a tuple of strings: `('a',)`, `('a', 'b')`.
pub(crate) fn tuple_repr(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| crate::pyrepr::repr_str(s)).collect();
    match parts.len() {
        0 => "()".into(),
        1 => format!("({},)", parts[0]),
        _ => format!("({})", parts.join(", ")),
    }
}

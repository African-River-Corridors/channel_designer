//! Channel sizing rules. Mirrors the `rules/` package of the frozen Python: `methods` (the width
//! method registry), `rulesets` (named standards, `channel_shape`, `size_channel`), `design`
//! (per-chainage sizing along a centreline), `simple` (one-call sizing) and `width_profile`.
//!
//! Vessel classes and the `Check` type come from `ovcd-core`, pinned to a tag (ADR 0001).
//!
//! Rulesets and width methods stay pluggable from outside the crate: a [`methods::WidthMethod`]
//! holds plain closures and a [`rulesets::Ruleset`] is data, and both live in a registry value
//! that the sizing functions take by reference. Python keeps the two registries in module
//! globals; here [`methods::global_methods`] and [`rulesets::global_rulesets`] are the same
//! thing for a caller that wants one process-wide registry, and any other registry value works
//! just as well (the parity runner makes a fresh one per case).

pub mod design;
pub mod methods;
pub mod rulesets;
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

//! Alignment: the line work between the river and the design line. One file per Python module of
//! `channel_designer.alignment` (`radius_check`, `loops`, `rolling_circle`, ...).

pub mod bends;
pub mod centreline;
pub mod limits;
pub mod loops;
pub mod meanders;
pub mod navigable;
pub mod radius_check;
pub mod rolling_circle;
pub mod route;
pub mod widths;

/// A refusal the Python raises: its exception type and its message, as the frozen driver records
/// them (`{"error": {"type": ..., "message": ...}}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyError {
    /// `"ValueError"`, `"IndexError"`, ...
    pub kind: &'static str,
    pub message: String,
}

impl PyError {
    pub fn value(message: impl Into<String>) -> Self {
        PyError {
            kind: "ValueError",
            message: message.into(),
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        PyError {
            kind: "RuntimeError",
            message: message.into(),
        }
    }

    pub fn index(message: impl Into<String>) -> Self {
        PyError {
            kind: "IndexError",
            message: message.into(),
        }
    }
}

impl core::fmt::Display for PyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for PyError {}

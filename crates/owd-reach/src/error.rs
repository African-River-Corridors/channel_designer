//! Errors of reach generation.

use crate::fetch::FetchError;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ReachError {
    Fetch(FetchError),
    /// A response that is not what the source documents.
    Parse(String),
    /// A click or trace point with no waterway within the snap distance.
    NoWaterway(String),
    /// Two points the waterway graph does not join.
    NoRoute(String),
    /// A bad argument.
    Input(String),
}

impl fmt::Display for ReachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReachError::Fetch(e) => write!(f, "{e}"),
            ReachError::Parse(m)
            | ReachError::NoWaterway(m)
            | ReachError::NoRoute(m)
            | ReachError::Input(m) => {
                write!(f, "{m}")
            }
        }
    }
}

impl std::error::Error for ReachError {}

impl From<FetchError> for ReachError {
    fn from(e: FetchError) -> Self {
        ReachError::Fetch(e)
    }
}

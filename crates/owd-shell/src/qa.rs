//! The QA overlay of a study (spec 0013, ticket T7): the banks, the centreline and the low-confidence
//! stretches over the Sentinel-2 true colour of the scenes the banks came from, as a PNG and a KMZ
//! for the app to save. The drawing is `owd_reach::overlay::render_document`, the same function
//! `owd qa-overlay` runs, so the app and the CLI give the same bytes. A job is made under the lock
//! and run off it, as the drawing sheets are: it reads the scenes over the network.

use crate::reach::ReachEndpoints;
use crate::state::AppState;
use owd_reach::Fetch;
use owd_reach::overlay::{OverlayRequest, render_document};
use serde_json::Value;

/// The document to draw, copied off the session.
pub struct QaJob {
    doc: Value,
}

impl AppState {
    /// The QA overlay's inputs, to run off the lock. Refused before the study has banks.
    pub fn qa_job(&self) -> Result<QaJob, String> {
        let doc = self.session().document().clone();
        if !doc["inputs"]["centreline"]["coordinates"].is_array() {
            return Err("make the bank lines first: the overlay draws them".into());
        }
        Ok(QaJob { doc })
    }
}

impl QaJob {
    /// The PNG (with its legend) and the KMZ, by file name, for the stretch between two chainages of
    /// the centreline in km (the whole of it where not given).
    pub fn files(
        &self,
        fetch: &dyn Fetch,
        at: &ReachEndpoints,
        from_km: Option<f64>,
        to_km: Option<f64>,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        let req = OverlayRequest {
            from_m: from_km.map(|k| k * 1000.0),
            to_m: to_km.map(|k| k * 1000.0),
            scenes: Vec::new(),
        };
        let r = render_document(fetch, &self.doc, &at.stac, &req).map_err(|e| e.to_string())?;
        let base = file_base(&r.name);
        Ok(vec![
            (format!("{base} QA overlay.png"), r.png),
            (format!("{base} QA overlay.kmz"), r.kmz),
        ])
    }
}

/// A file name from a free title: characters a file system refuses become spaces.
fn file_base(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|".contains(c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn a_title_becomes_a_safe_file_name() {
        assert_eq!(file_base("Sassandra: km 1/2 to 3"), "Sassandra km 1 2 to 3");
        assert!(AppState::new_design("t", "2026-10-22").qa_job().is_err());
    }
}

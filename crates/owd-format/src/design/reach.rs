//! The reach a study was started from (spec 0012): the user's start, end, via points and trace,
//! and the route as OSM way ids. OSM geometry is never stored (ODbL): a stretch with no waterway
//! near it is kept as the user drew it, as `as-drawn`.

use super::session::{DesignError, Session};
use serde_json::Value;

impl Session {
    /// Record the reach. The value must fit the schema's `reach`; on a refusal the document is
    /// unchanged.
    pub fn set_reach(&mut self, reach: Value) -> Result<(), DesignError> {
        let before = self.doc.get("reach").cloned();
        self.doc["reach"] = reach;
        let errors = self.validate();
        if errors.is_empty() {
            return Ok(());
        }
        match before {
            Some(b) => self.doc["reach"] = b,
            None => {
                self.doc.as_object_mut().map(|d| d.remove("reach"));
            }
        }
        Err(DesignError::Input(errors.join("; ")))
    }

    /// The recorded reach, if any.
    pub fn reach(&self) -> Option<&Value> {
        self.doc.get("reach").filter(|v| v.is_object())
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use serde_json::json;

    fn reach() -> Value {
        json!({
            "river": "Test", "start": [-6.0, 4.9], "end": [-6.1, 5.0], "via": [], "trace": null,
            "route": [{"kind": "waterway", "way_ids": [101, 102]},
                      {"kind": "as-drawn", "line": [[-6.05, 4.95], [-6.06, 4.96]]}]
        })
    }

    #[test]
    fn a_reach_round_trips_and_a_bad_one_changes_nothing() {
        let mut s = Session::new("t", "2026-10-21");
        s.set_reach(reach()).unwrap();
        let back = Session::open(&s.to_text()).unwrap();
        assert_eq!(
            back.reach().unwrap()["route"][0]["way_ids"],
            json!([101, 102])
        );
        let mut bad = reach();
        bad["route"][0]["kind"] = json!("road");
        assert!(s.set_reach(bad).is_err());
        assert_eq!(s.reach().unwrap()["route"][0]["kind"], "waterway");
        let mut empty = Session::new("t", "2026-10-21");
        assert!(empty.set_reach(json!({"start": [0, 0]})).is_err());
        assert!(empty.reach().is_none());
    }
}

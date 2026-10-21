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

    /// Record the stretches of generated banks that are low-confidence width. Replaces an earlier
    /// record; on a refusal the document is unchanged.
    pub fn set_low_confidence_width(&mut self, value: Value) -> Result<(), DesignError> {
        let before = self.doc.get("low_confidence_width").cloned();
        self.doc["low_confidence_width"] = value;
        let errors = self.validate();
        if errors.is_empty() {
            return Ok(());
        }
        match before {
            Some(b) => self.doc["low_confidence_width"] = b,
            None => {
                self.doc
                    .as_object_mut()
                    .map(|d| d.remove("low_confidence_width"));
            }
        }
        Err(DesignError::Input(errors.join("; ")))
    }

    /// The recorded low-confidence stretches, if any.
    pub fn low_confidence_width(&self) -> Option<&Value> {
        self.doc
            .get("low_confidence_width")
            .filter(|v| v.is_object())
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

    #[test]
    fn low_confidence_stretches_round_trip_and_a_bad_kind_is_refused() {
        let mut s = Session::new("t", "2026-10-21");
        let v = json!({"min_width_m": 90.0, "provisional": true, "source": "jrc-gsw",
                       "stretches": [{"from_m": 100.0, "to_m": 250.0, "kind": "narrow", "min_width_m": 60.0}]});
        s.set_low_confidence_width(v).unwrap();
        let back = Session::open(&s.to_text()).unwrap();
        assert_eq!(
            back.low_confidence_width().unwrap()["stretches"][0]["kind"],
            "narrow"
        );
        let bad =
            json!({"min_width_m": 90.0, "stretches": [{"from_m": 0, "to_m": 1, "kind": "wide"}]});
        assert!(s.set_low_confidence_width(bad).is_err());
        assert_eq!(
            s.low_confidence_width().unwrap()["stretches"][0]["from_m"],
            100.0
        );
    }
}

//! Provenance of generated inputs (spec 0012, ADR 0010): for each input OWD made, the source it came
//! from, its date, the licence notice and a mark. The notices travel into every export. The record
//! sits beside `inputs`, not in it, so it never changes a stage's dependency hash: a document with
//! or without it gives the same results.

use super::session::{DesignError, Session};
use serde_json::{Value, json};

/// The inputs a record can describe.
pub const INPUT_NAMES: [&str; 8] = [
    "left_bank",
    "right_bank",
    "centreline",
    "river_polygon",
    "dem",
    "water_level",
    "route",
    "bed",
];

/// Who made an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Made by OWD from open data.
    Default,
    /// A default the user changed.
    Edited,
    /// The user's own.
    User,
}

impl Mark {
    pub fn name(self) -> &'static str {
        match self {
            Mark::Default => "default",
            Mark::Edited => "edited",
            Mark::User => "user",
        }
    }
}

/// One provenance record as a JSON value.
pub fn record(source: &str, date: &str, notice: &str, mark: Mark, provisional: bool) -> Value {
    let mut r = json!({"source": source, "date": date, "notice": notice, "mark": mark.name()});
    if provisional {
        r["provisional"] = json!(true);
    }
    r
}

impl Session {
    /// Record where `input` came from. Replaces an earlier record for it. Fails on an unknown input
    /// name or a record that breaks the schema, and then changes nothing.
    pub fn set_provenance(&mut self, input: &str, rec: Value) -> Result<(), DesignError> {
        if !INPUT_NAMES.contains(&input) {
            return Err(DesignError::Input(format!(
                "`{input}` is not an input with provenance (one of {})",
                INPUT_NAMES.join(", ")
            )));
        }
        let before = self.doc["provenance"].clone();
        if !self.doc["provenance"].is_object() {
            self.doc["provenance"] = json!({});
        }
        self.doc["provenance"][input] = rec;
        let errors = self.validate();
        if errors.is_empty() {
            Ok(())
        } else {
            match before {
                Value::Null => {
                    self.doc.as_object_mut().map(|d| d.remove("provenance"));
                }
                b => self.doc["provenance"] = b,
            }
            Err(DesignError::Input(errors.join("; ")))
        }
    }

    /// The provenance record of one input, if the document has one.
    pub fn provenance(&self, input: &str) -> Option<&Value> {
        self.doc["provenance"].get(input).filter(|v| v.is_object())
    }

    /// Every licence notice of the inputs in use, once each, in input order.
    pub fn notices(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for name in INPUT_NAMES {
            if let Some(n) = self.provenance(name).and_then(|r| r["notice"].as_str())
                && !out.iter().any(|o| o == n)
            {
                out.push(n.to_string());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    fn rec(notice: &str) -> Value {
        record("jrc-gsw", "2021-12-31", notice, Mark::Default, true)
    }

    #[test]
    fn a_document_with_provenance_validates_and_round_trips() {
        let mut s = Session::new("t", "2026-10-21");
        s.set_provenance("left_bank", rec("Source: EC JRC/Google"))
            .unwrap();
        s.set_provenance(
            "dem",
            record(
                "cop-dem-glo30",
                "2026-10-21",
                "Copernicus DEM",
                Mark::User,
                false,
            ),
        )
        .unwrap();
        let text = s.to_text();
        let back = Session::open(&text).unwrap();
        assert_eq!(back.provenance("left_bank").unwrap()["mark"], "default");
        assert_eq!(back.provenance("left_bank").unwrap()["provisional"], true);
        assert_eq!(back.provenance("dem").unwrap()["mark"], "user");
        assert!(back.validate().is_empty());
    }

    #[test]
    fn a_bad_record_or_name_is_refused_and_changes_nothing() {
        let mut s = Session::new("t", "2026-10-21");
        assert!(s.set_provenance("nope", rec("n")).is_err());
        assert!(
            s.set_provenance(
                "dem",
                json!({"source": "x", "date": "2026", "notice": "n", "mark": "guess"})
            )
            .is_err()
        );
        assert!(s.doc.get("provenance").is_none());
        assert!(s.notices().is_empty());
    }

    #[test]
    fn notices_are_listed_once_each() {
        let mut s = Session::new("t", "2026-10-21");
        s.set_provenance("left_bank", rec("Same notice")).unwrap();
        s.set_provenance("right_bank", rec("Same notice")).unwrap();
        s.set_provenance("dem", rec("Other notice")).unwrap();
        assert_eq!(s.notices(), vec!["Same notice", "Other notice"]);
    }
}

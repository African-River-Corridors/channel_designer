//! Provenance records for generated inputs: the registry entry, the date and the mark, as the
//! document stores them (`owd_format::design::provenance`).

use crate::sources::source;
use owd_format::design::provenance::{Mark, record};
use serde_json::Value;

/// The record for an input made from source `source_id` on `date` (`YYYY-MM-DD`). `provisional`
/// marks a default that is not yet calibrated (spec 0012). Fails for an id not in the registry.
pub fn provenance_record(
    source_id: &str,
    date: &str,
    mark: Mark,
    provisional: bool,
) -> Result<Value, String> {
    let s = source(source_id).ok_or_else(|| format!("`{source_id}` is not a known source"))?;
    Ok(record(
        source_id,
        date,
        &s.notice_for(date),
        mark,
        provisional,
    ))
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;
    use owd_format::design::Session;

    #[test]
    fn a_registry_record_goes_into_a_document_and_its_notice_into_the_exports() {
        let mut d = Session::new("t", "2026-10-21");
        let r = provenance_record("sentinel2-l2a", "2025-02-14", Mark::Default, true).unwrap();
        d.set_provenance("left_bank", r).unwrap();
        assert_eq!(
            d.notices(),
            vec!["Contains modified Copernicus Sentinel data 2025"]
        );
        assert!(provenance_record("nope", "2026-01-01", Mark::User, false).is_err());
    }
}

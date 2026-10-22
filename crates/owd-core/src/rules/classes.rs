//! Vessel classes from the shared catalogue (`ovcd-core`, spec 0015 T0): the classes a channel can be
//! sized for, their design figures, the 12 classes `fits` checks by default, and the ids older studies
//! hold.
//!
//! A class can be picked when it has a length, a beam, a draught and a formation the catalogue maps to
//! a vessel kind. The design figures are as before: the long end of the length range, the beam, and the
//! middle of the draught range unless the user gives one.

use super::RuleError;
use ovcd_core::vessel_classes::{CLASSES, ClassRow, LEGACY_IDS, find};
use ovcd_core::vessel_standards::{VesselStandard, get_standard};

/// The 12 classes OWD offered before the full catalogue, by catalogue id, in their old order: what
/// `owd fits` checks unless asked for every class.
pub fn default_class_ids() -> Vec<&'static str> {
    LEGACY_IDS.iter().map(|(_, id)| *id).collect()
}

/// The catalogue id of a class id: an id older studies hold (`CEMT_Va`) gives its catalogue id
/// (`CEMT-Va`). `None` for an id the catalogue does not know.
pub fn canonical_id(id: &str) -> Option<&'static str> {
    find(id).map(|c| c.id)
}

/// Why a class cannot be picked, naming what it lacks; `None` when it can.
fn not_pickable(c: &ClassRow) -> Option<String> {
    let missing: Vec<&str> = [
        (c.loa_m.is_none(), "length"),
        (c.beam_m.is_none(), "beam"),
        (c.draught_m.is_none(), "draught"),
    ]
    .into_iter()
    .filter_map(|(gone, what)| gone.then_some(what))
    .collect();
    let mut why = Vec::new();
    if !missing.is_empty() {
        why.push(format!(
            "the catalogue gives it no {}",
            missing.join(", no ")
        ));
    }
    if c.kind().is_none() {
        why.push(format!(
            "its formation ({}) is not one a channel is sized for",
            c.formation.replace('_', " ")
        ));
    }
    (!why.is_empty()).then(|| {
        format!(
            "vessel class {} cannot size a channel: {}",
            c.id,
            why.join("; ")
        )
    })
}

/// Every class a channel can be sized for, in catalogue order.
pub fn pickable_classes() -> Vec<VesselStandard> {
    CLASSES
        .iter()
        .filter(|c| not_pickable(c).is_none())
        .map(VesselStandard::from)
        .collect()
}

/// The class for an id (a catalogue id or an older one), refused with a message that names what it
/// lacks when it cannot size a channel.
pub fn class(id: &str) -> Result<VesselStandard, RuleError> {
    if let Some(c) = find(id)
        && let Some(why) = not_pickable(c)
    {
        return Err(RuleError::Value(why));
    }
    get_standard(id).map_err(RuleError::Key)
}

/// The figures a channel is sized on: the long end of the length range, the beam, and the draught
/// (the middle of the class's range unless `draught_m` is given).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignFigures {
    pub loa_m: f64,
    pub beam_m: f64,
    pub draught_m: f64,
}

/// The design figures of a class that can be picked (see [`class`]).
pub fn design_figures(v: &VesselStandard, draught_m: Option<f64>) -> DesignFigures {
    DesignFigures {
        loa_m: v.loa_m.map_or(f64::NAN, |(_, hi)| hi),
        beam_m: v.beam_m.unwrap_or(f64::NAN),
        draught_m: draught_m
            .or_else(|| v.draught_nominal_m())
            .unwrap_or(f64::NAN),
    }
}

/// The top of a class's deadweight range in the catalogue, in tonnes.
pub fn deadweight_top_t(id: &str) -> Option<f64> {
    find(id).and_then(|c| c.dwt_t).map(|(_, hi)| hi)
}

#[cfg(test)]
mod tests {
    //! Label: sourced - `ovcd-core` v0.2.4, `catalogue/vessel_classes.yaml` (the shared catalogue).
    use super::*;

    #[test]
    fn older_ids_find_their_catalogue_class() {
        assert_eq!(canonical_id("CEMT_Va"), Some("CEMT-Va"));
        assert_eq!(canonical_id("CN_III_convoy_2ab"), Some("CN-III-CONVOY-2x2"));
        assert_eq!(canonical_id("CEMT-Va"), Some("CEMT-Va"));
        assert_eq!(canonical_id("nope"), None);
        assert_eq!(default_class_ids().len(), 12);
        assert_eq!(default_class_ids()[0], "CEMT-IV");
    }

    #[test]
    fn every_pickable_class_has_figures_and_a_class_without_them_is_refused_by_name() {
        let all = pickable_classes();
        assert!(all.len() > 12, "{}", all.len());
        for v in &all {
            let f = design_figures(v, None);
            assert!(
                f.loa_m > f.beam_m && f.beam_m > 0.0 && f.draught_m > 0.0,
                "{v:?}"
            );
        }
        assert!(all.iter().any(|v| v.standard_id == "CEMT-VIb-motor"));
        let refused = CLASSES
            .iter()
            .find(|c| not_pickable(c).is_some())
            .expect("the catalogue has classes that cannot size a channel");
        let e = class(refused.id).unwrap_err();
        assert!(e.raw_message().contains(refused.id), "{e:?}");
        assert!(e.raw_message().contains("cannot size a channel"), "{e:?}");
        assert!(matches!(class("nope"), Err(RuleError::Key(_))));
    }
}

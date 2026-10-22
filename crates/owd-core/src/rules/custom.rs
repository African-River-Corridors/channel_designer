//! A custom vessel (spec 0015 T1): a vessel of the user's own, sized like a catalogue class and
//! labelled as custom everywhere.
//!
//! The checks:
//! - **Refused,** naming the field: a dimension that is zero, negative or not a number; a beam not
//!   less than the length; an empty name or one over 60 characters; a convoy with no `n_abreast` or
//!   `n_in_line` (or a motor vessel with one); a GB class that is not `I` to `VII`.
//! - **Warned,** and the run goes on: a length, beam or draught outside the range of every class a
//!   channel can be sized for; a length-to-beam ratio below [`LB_RATIO_MIN`] or above
//!   [`LB_RATIO_MAX`].
//! - **Never refused:** a draught deeper than the river. It shows as dredging, as for any class.
//!
//! GB 50139 cannot derive a class for a vessel it does not know, so it reads the typed `gb_class`.
//! With none, its `needs-class` Rules mark the width and depth "not evaluated". A custom vessel is
//! always a special design vessel (GB 50139-2014 Appendix A), never a typical one.

use super::RuleError;
use super::engine::Vessel;

/// The family of every custom vessel, as the engine reads it.
pub const CUSTOM_FAMILY: &str = "custom";

/// The longest name, in characters.
pub const NAME_MAX_CHARS: usize = 60;

/// Below this length-to-beam ratio a vessel is unusual: a warning. Provisional.
pub const LB_RATIO_MIN: f64 = 2.5;
/// Above this length-to-beam ratio a vessel is unusual: a warning. Provisional.
pub const LB_RATIO_MAX: f64 = 15.0;

/// The GB 50139 channel classes a custom vessel may be given.
pub const GB_CLASSES: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

/// How the vessel travels: a self-propelled vessel or a pushed convoy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formation {
    Motor,
    Convoy,
}

impl Formation {
    /// `motor` or `convoy`, as the study and the CLI write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Formation::Motor => "motor",
            Formation::Convoy => "convoy",
        }
    }

    pub fn parse(s: &str) -> Result<Formation, RuleError> {
        match s {
            "motor" => Ok(Formation::Motor),
            "convoy" => Ok(Formation::Convoy),
            _ => Err(RuleError::Value(format!(
                "the formation must be `motor` or `convoy`, not `{s}`"
            ))),
        }
    }

    /// The engine's word: `motor_vessel` or `pushed_convoy`.
    pub fn engine_word(self) -> &'static str {
        match self {
            Formation::Motor => "motor_vessel",
            Formation::Convoy => "pushed_convoy",
        }
    }
}

/// A custom vessel. For a convoy, `loa_m` and `beam_m` are those of the whole convoy.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomVessel {
    pub name: String,
    pub loa_m: f64,
    pub beam_m: f64,
    pub draught_m: f64,
    pub formation: Formation,
    pub n_abreast: Option<u32>,
    pub n_in_line: Option<u32>,
    /// The GB 50139 channel class, `I` to `VII`; `None` when the user gives none.
    pub gb_class: Option<String>,
    /// Stored only (a later bridge check).
    pub air_draught_m: Option<f64>,
    /// Stored only.
    pub deadweight_t: Option<f64>,
}

/// A number in its shortest form: `140`, `4.7`.
fn short(x: f64) -> String {
    format!("{x}")
}

/// A refusal of a custom vessel: the field it names (as the study writes it, such as `beam_m`)
/// and the message.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub field: &'static str,
    pub message: String,
}

fn refuse(field: &'static str, message: String) -> Option<Refusal> {
    Some(Refusal { field, message })
}

/// A refusal of `x` unless it is a positive number. `what` names it in words.
fn positive(field: &'static str, what: &str, x: f64) -> Option<Refusal> {
    if x.is_finite() && x > 0.0 {
        None
    } else {
        refuse(
            field,
            format!("the custom vessel's {what} ({field}) must be a positive number, not {x}"),
        )
    }
}

/// The lowest and highest of a dimension over every class a channel can be sized for.
fn envelope(
    of: impl Fn(&ovcd_core::vessel_standards::VesselStandard) -> Option<(f64, f64)>,
) -> (f64, f64) {
    super::classes::pickable_classes()
        .iter()
        .filter_map(of)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (a, b)| {
            (lo.min(a), hi.max(b))
        })
}

impl CustomVessel {
    /// The label every output shows: `Custom: My barge, 140 × 29 × 4.7 m`.
    pub fn label(&self) -> String {
        format!(
            "Custom: {}, {} × {} × {} m",
            self.name.trim(),
            short(self.loa_m),
            short(self.beam_m),
            short(self.draught_m)
        )
    }

    /// Refuse a vessel that cannot be right, naming the field; else the warnings (possibly none).
    pub fn check(&self) -> Result<Vec<String>, RuleError> {
        match self.refusal() {
            Some(r) => Err(RuleError::Value(r.message)),
            None => Ok(self.warnings()),
        }
    }

    /// The first refusal of the vessel, with the field it names; `None` when it can be sized.
    pub fn refusal(&self) -> Option<Refusal> {
        let name = self.name.trim();
        if name.is_empty() {
            return refuse("name", "the custom vessel's name must not be empty".into());
        }
        if name.chars().count() > NAME_MAX_CHARS {
            return refuse(
                "name",
                format!("the custom vessel's name must be at most {NAME_MAX_CHARS} characters"),
            );
        }
        for (field, what, x) in [
            ("loa_m", "length overall", self.loa_m),
            ("beam_m", "beam", self.beam_m),
            ("draught_m", "draught", self.draught_m),
        ] {
            if let Some(r) = positive(field, what, x) {
                return Some(r);
            }
        }
        if self.beam_m >= self.loa_m {
            return refuse(
                "beam_m",
                format!(
                    "the custom vessel's beam (beam_m, {} m) must be less than its length \
                     overall (loa_m, {} m)",
                    short(self.beam_m),
                    short(self.loa_m)
                ),
            );
        }
        match self.formation {
            Formation::Convoy => {
                for (field, n) in [("n_abreast", self.n_abreast), ("n_in_line", self.n_in_line)] {
                    if !n.is_some_and(|n| n >= 1) {
                        return refuse(
                            field,
                            format!(
                                "a custom convoy needs {field}: the number of barges, at least 1"
                            ),
                        );
                    }
                }
            }
            Formation::Motor => {
                if let Some(field) = [("n_abreast", self.n_abreast), ("n_in_line", self.n_in_line)]
                    .into_iter()
                    .find_map(|(f, n)| n.map(|_| f))
                {
                    return refuse(
                        field,
                        "n_abreast and n_in_line are for a convoy only".into(),
                    );
                }
            }
        }
        if let Some(c) = &self.gb_class
            && !GB_CLASSES.contains(&c.as_str())
        {
            return refuse(
                "gb_class",
                format!(
                    "the custom vessel's gb_class must be one of {}, not `{c}`",
                    GB_CLASSES.join(", ")
                ),
            );
        }
        if let Some(r) = self
            .air_draught_m
            .and_then(|a| positive("air_draught_m", "air draught", a))
        {
            return Some(r);
        }
        self.deadweight_t
            .and_then(|d| positive("deadweight_t", "deadweight", d))
    }

    /// The warnings of a vessel that can be sized: outside every catalogue class, or an unusual
    /// length-to-beam ratio.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let dims = [
            ("length overall", self.loa_m, envelope(|v| v.loa_m)),
            ("beam", self.beam_m, envelope(|v| v.beam_m.map(|b| (b, b)))),
            ("draught", self.draught_m, envelope(|v| v.draught_m)),
        ];
        for (what, x, (lo, hi)) in dims {
            if x < lo || x > hi {
                warnings.push(format!(
                    "the custom vessel's {what} ({} m) is outside every catalogue class ({} to {} m)",
                    short(x),
                    short(lo),
                    short(hi)
                ));
            }
        }
        let ratio = self.loa_m / self.beam_m;
        if !(LB_RATIO_MIN..=LB_RATIO_MAX).contains(&ratio) {
            warnings.push(format!(
                "the custom vessel's length-to-beam ratio ({ratio:.1}) is outside {LB_RATIO_MIN} \
                 to {LB_RATIO_MAX}"
            ));
        }
        warnings
    }

    /// The vessel the engine sizes. Its class is the label; its family is [`CUSTOM_FAMILY`].
    pub fn vessel(&self) -> Vessel {
        Vessel {
            loa_m: self.loa_m,
            beam_m: self.beam_m,
            draught_m: self.draught_m,
            formation: self.formation.engine_word().into(),
            class_name: self.label(),
            family: CUSTOM_FAMILY.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Label: property (each refusal and each warning raised by a vessel built to break it).
    use super::*;

    fn barge() -> CustomVessel {
        CustomVessel {
            name: "My barge".into(),
            loa_m: 140.0,
            beam_m: 29.0,
            draught_m: 4.7,
            formation: Formation::Motor,
            n_abreast: None,
            n_in_line: None,
            gb_class: None,
            air_draught_m: None,
            deadweight_t: None,
        }
    }

    fn refused(v: CustomVessel) -> String {
        v.check().unwrap_err().raw_message().to_string()
    }

    #[test]
    fn the_label_names_the_vessel_and_its_dimensions() {
        assert_eq!(barge().label(), "Custom: My barge, 140 × 29 × 4.7 m");
    }

    #[test]
    fn a_vessel_like_a_catalogue_class_has_no_warning() {
        let v = CustomVessel {
            loa_m: 110.0,
            beam_m: 11.4,
            draught_m: 2.65,
            ..barge()
        };
        assert_eq!(v.check().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn each_bad_dimension_is_refused_by_name() {
        for (v, field) in [
            (
                CustomVessel {
                    loa_m: 0.0,
                    ..barge()
                },
                "loa_m",
            ),
            (
                CustomVessel {
                    beam_m: -1.0,
                    ..barge()
                },
                "beam_m",
            ),
            (
                CustomVessel {
                    draught_m: f64::NAN,
                    ..barge()
                },
                "draught_m",
            ),
            (
                CustomVessel {
                    air_draught_m: Some(0.0),
                    ..barge()
                },
                "air_draught_m",
            ),
            (
                CustomVessel {
                    deadweight_t: Some(-5.0),
                    ..barge()
                },
                "deadweight_t",
            ),
        ] {
            let m = refused(v);
            assert!(m.contains(field), "{field}: {m}");
        }
    }

    #[test]
    fn a_beam_not_less_than_the_length_is_refused() {
        let m = refused(CustomVessel {
            beam_m: 140.0,
            ..barge()
        });
        assert!(m.contains("beam_m") && m.contains("less than"), "{m}");
    }

    #[test]
    fn an_empty_or_long_name_is_refused() {
        assert!(
            refused(CustomVessel {
                name: "  ".into(),
                ..barge()
            })
            .contains("name")
        );
        assert!(
            refused(CustomVessel {
                name: "x".repeat(61),
                ..barge()
            })
            .contains("60")
        );
        assert!(
            CustomVessel {
                name: "x".repeat(60),
                ..barge()
            }
            .check()
            .is_ok()
        );
    }

    #[test]
    fn a_convoy_needs_its_barge_counts_and_a_motor_vessel_has_none() {
        let convoy = CustomVessel {
            formation: Formation::Convoy,
            n_abreast: Some(2),
            n_in_line: Some(2),
            ..barge()
        };
        assert!(convoy.check().is_ok());
        assert!(
            refused(CustomVessel {
                n_abreast: None,
                ..convoy.clone()
            })
            .contains("n_abreast")
        );
        assert!(
            refused(CustomVessel {
                n_in_line: Some(0),
                ..convoy
            })
            .contains("n_in_line")
        );
        assert!(
            refused(CustomVessel {
                n_abreast: Some(2),
                ..barge()
            })
            .contains("convoy only")
        );
    }

    #[test]
    fn a_gb_class_outside_i_to_vii_is_refused() {
        assert!(
            refused(CustomVessel {
                gb_class: Some("VIII".into()),
                ..barge()
            })
            .contains("gb_class")
        );
        assert!(
            CustomVessel {
                gb_class: Some("III".into()),
                ..barge()
            }
            .check()
            .is_ok()
        );
    }

    #[test]
    fn a_vessel_outside_every_class_is_warned_not_refused() {
        let w = CustomVessel {
            loa_m: 600.0,
            beam_m: 29.0,
            ..barge()
        }
        .check()
        .unwrap();
        assert!(
            w.iter().any(
                |w| w.contains("length overall") && w.contains("outside every catalogue class")
            ),
            "{w:?}"
        );
    }

    #[test]
    fn an_unusual_length_to_beam_ratio_is_warned() {
        let w = CustomVessel {
            loa_m: 60.0,
            beam_m: 29.0,
            ..barge()
        }
        .check()
        .unwrap();
        assert!(
            w.iter().any(|w| w.contains("length-to-beam ratio (2.1)")),
            "{w:?}"
        );
        let w = CustomVessel {
            loa_m: 140.0,
            beam_m: 9.0,
            ..barge()
        }
        .check()
        .unwrap();
        assert!(
            w.iter().any(|w| w.contains("length-to-beam ratio (15.6)")),
            "{w:?}"
        );
    }

    /// The app shows a refusal next to its field (spec 0015 T2): each names the field it refuses.
    #[test]
    fn each_refusal_names_its_field() {
        for (v, field) in [
            (
                CustomVessel {
                    name: " ".into(),
                    ..barge()
                },
                "name",
            ),
            (
                CustomVessel {
                    loa_m: 0.0,
                    ..barge()
                },
                "loa_m",
            ),
            (
                CustomVessel {
                    beam_m: 200.0,
                    ..barge()
                },
                "beam_m",
            ),
            (
                CustomVessel {
                    draught_m: -1.0,
                    ..barge()
                },
                "draught_m",
            ),
            (
                CustomVessel {
                    formation: Formation::Convoy,
                    n_in_line: Some(2),
                    ..barge()
                },
                "n_abreast",
            ),
            (
                CustomVessel {
                    n_in_line: Some(2),
                    ..barge()
                },
                "n_in_line",
            ),
            (
                CustomVessel {
                    gb_class: Some("X".into()),
                    ..barge()
                },
                "gb_class",
            ),
            (
                CustomVessel {
                    air_draught_m: Some(0.0),
                    ..barge()
                },
                "air_draught_m",
            ),
            (
                CustomVessel {
                    deadweight_t: Some(f64::NAN),
                    ..barge()
                },
                "deadweight_t",
            ),
        ] {
            assert_eq!(v.refusal().map(|r| r.field), Some(field));
        }
        assert_eq!(barge().refusal(), None);
    }

    #[test]
    fn a_very_deep_draught_is_allowed() {
        assert!(
            CustomVessel {
                draught_m: 9.0,
                ..barge()
            }
            .check()
            .is_ok()
        );
    }

    #[test]
    fn the_engine_vessel_is_custom_with_the_engine_formation() {
        let v = CustomVessel {
            formation: Formation::Convoy,
            n_abreast: Some(2),
            n_in_line: Some(2),
            ..barge()
        }
        .vessel();
        assert_eq!(v.family, CUSTOM_FAMILY);
        assert_eq!(v.formation, "pushed_convoy");
        assert_eq!(v.class_name, "Custom: My barge, 140 × 29 × 4.7 m");
        assert_eq!((v.loa_m, v.beam_m, v.draught_m), (140.0, 29.0, 4.7));
    }
}

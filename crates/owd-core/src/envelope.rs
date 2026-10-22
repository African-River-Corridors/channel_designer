//! The largest vessel the river allows (spec 0016 T2, `owd envelope`).
//!
//! A fixed series of single-hull motor vessels, each with a length-to-beam ratio of 5 by default
//! ([`DEFAULT_SIZES`]), all at one draught ([`DEFAULT_DRAUGHT_M`]), is sized under one Rule Set
//! choice (PIANC WG 141 quality C by default). For each size, its own fairway fit (ADR 0007) is
//! seeded from the reference line, in the water less half its bottom width, with its own R_min, and
//! the width and the radius are read on that fitted line, as `fits` reads them. The answers:
//! the largest size with no blocked stretch, and the largest size that passes each kilometre of the
//! reference line. Plan only: width and radius. No file, clock or network access.

use crate::alignment::PyError;
use crate::alignment::fairway_fit::{FitParams, Fixed, fairway_fit_in_water};
use crate::alignment::navigable::Chained;
use crate::fits::{FitStretch, class_fit, classes_in_size_order};
use crate::geom::{Polygon, Pt, linear};
use crate::rules::custom::{CustomVessel, Formation};
use crate::rules::engine::selection_for_old_ruleset;
use crate::rules::rulesets::channel_shape_for_custom;

/// The default series: length × beam (m).
pub const DEFAULT_SIZES: [(f64, f64); 5] = [
    (10.0, 2.0),
    (20.0, 4.0),
    (30.0, 6.0),
    (40.0, 8.0),
    (50.0, 10.0),
];
/// The draught of every size (m), unless set.
pub const DEFAULT_DRAUGHT_M: f64 = 2.0;
/// The Rule Set choice, unless set (an old ruleset name).
pub const DEFAULT_RULE_SET: &str = "PIANC-C";
/// The length of a piece of the per-kilometre strip (m).
pub const KM_M: f64 = 1000.0;

/// One size of the series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub loa_m: f64,
    pub beam_m: f64,
}

impl Size {
    /// `30x6`, as `--sizes` writes it.
    pub fn label(&self) -> String {
        format!("{}x{}", self.loa_m, self.beam_m)
    }

    /// Read `10x2,20x4,…`. Each length and beam must be a positive number.
    pub fn parse_list(s: &str) -> Result<Vec<Size>, String> {
        s.split(',')
            .map(|p| {
                let (l, b) = p
                    .trim()
                    .split_once(['x', 'X', '×'])
                    .ok_or_else(|| format!("a size is `LENGTHxBEAM`, such as `30x6`, not `{p}`"))?;
                let num = |t: &str| {
                    t.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .ok_or_else(|| format!("`{t}` in `{p}` is not a positive number"))
                };
                Ok(Size {
                    loa_m: num(l)?,
                    beam_m: num(b)?,
                })
            })
            .collect()
    }
}

/// The default series as [`Size`]s.
pub fn default_sizes() -> Vec<Size> {
    DEFAULT_SIZES
        .iter()
        .map(|&(loa_m, beam_m)| Size { loa_m, beam_m })
        .collect()
}

/// The settings of a sweep.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub sizes: Vec<Size>,
    pub draught_m: f64,
    /// An old ruleset name, such as `PIANC-C`.
    pub rule_set: String,
    /// A clearance from the banks on top of half the bottom width (m), 0 by default.
    pub clearance_m: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sizes: default_sizes(),
            draught_m: DEFAULT_DRAUGHT_M,
            rule_set: DEFAULT_RULE_SET.into(),
            clearance_m: 0.0,
        }
    }
}

/// The answer for one size. Its stretches are by chainage on the reference line.
#[derive(Debug, Clone, PartialEq)]
pub struct SizeResult {
    pub size: Size,
    pub r_min_m: f64,
    pub bottom_width_straight_m: f64,
    /// True when no stretch blocks the size.
    pub passes: bool,
    pub stretches: Vec<FitStretch>,
    /// The size's fitted line.
    pub line: Vec<Pt>,
    pub notes: Vec<String>,
}

/// One kilometre of the reference line, and the largest size that passes it (an index into the
/// series), if any.
#[derive(Debug, Clone, PartialEq)]
pub struct KmResult {
    pub from_m: f64,
    pub to_m: f64,
    pub largest: Option<usize>,
}

/// A catalogue class or the custom vessel, marked against the series by length and beam.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub id: String,
    pub custom: bool,
    pub loa_m: f64,
    pub beam_m: f64,
    /// True when it is no longer and no wider than the largest size that passes the whole reach.
    pub within: bool,
}

/// The whole answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub settings: Settings,
    pub length_m: f64,
    pub sizes: Vec<SizeResult>,
    /// The largest size (an index into the series) with no blocked stretch.
    pub largest: Option<usize>,
    pub per_km: Vec<KmResult>,
    pub marks: Vec<Mark>,
}

/// One size: its own fairway fit seeded from `reference` in `water`, and the width and radius read
/// on it.
pub fn size_result(
    reference: &[Pt],
    water: &[Polygon],
    settings: &Settings,
    size: Size,
) -> Result<SizeResult, PyError> {
    let selection = selection_for_old_ruleset(&settings.rule_set)
        .map_err(|e| PyError::value(e.raw_message().to_string()))?;
    let v = CustomVessel {
        name: size.label(),
        loa_m: size.loa_m,
        beam_m: size.beam_m,
        draught_m: settings.draught_m,
        formation: Formation::Motor,
        n_abreast: None,
        n_in_line: None,
        gb_class: None,
        air_draught_m: None,
        deadweight_t: None,
    };
    let shape = channel_shape_for_custom(&selection, &v)
        .map_err(|e| PyError::value(e.raw_message().to_string()))?;
    let r_min = shape.min_bend_radius_m;
    let fit = fairway_fit_in_water(
        reference,
        water,
        &shape,
        settings.clearance_m,
        0.0,
        &Fixed::default(),
        &FitParams::new(r_min),
    )?;
    let who = (size.label(), "series".to_string(), "motor_vessel".to_string());
    let row = class_fit(&Chained::new(&fit.line), water, &shape, who)?;
    // Onto the reference line, so the sizes and the strip share one chainage.
    let ix = linear::ProjectIndex::new(reference);
    let stretches: Vec<FitStretch> = row
        .stretches
        .into_iter()
        .map(|mut s| {
            let (a, b) = (ix.project(s.start), ix.project(s.end));
            (s.start_chainage_m, s.end_chainage_m) = (a.min(b), a.max(b));
            s
        })
        .collect();
    Ok(SizeResult {
        size,
        r_min_m: r_min,
        bottom_width_straight_m: shape.bottom_width_straight_m,
        passes: stretches.is_empty(),
        stretches,
        line: fit.line,
        notes: fit.notes,
    })
}

/// Put the sizes' answers together: the largest size, the strip, and the marks of the 12 default
/// classes and the custom vessel.
pub fn combine(
    reference: &[Pt],
    settings: Settings,
    sizes: Vec<SizeResult>,
    custom: Option<&CustomVessel>,
) -> Envelope {
    let length_m = linear::length(reference);
    let largest = sizes.iter().rposition(|s| s.passes);
    let n = (length_m / KM_M).ceil().max(1.0) as usize;
    let per_km = (0..n)
        .map(|k| {
            let (a, b) = (k as f64 * KM_M, ((k + 1) as f64 * KM_M).min(length_m));
            let blocked = |s: &SizeResult| {
                s.stretches
                    .iter()
                    .any(|st| st.start_chainage_m < b && st.end_chainage_m >= a)
            };
            KmResult {
                from_m: a,
                to_m: b,
                largest: sizes.iter().rposition(|s| !blocked(s)),
            }
        })
        .collect();
    let top = largest.map(|i| sizes[i].size);
    let within = |loa: f64, beam: f64| top.is_some_and(|t| loa <= t.loa_m && beam <= t.beam_m);
    let mut marks: Vec<Mark> = classes_in_size_order()
        .into_iter()
        .filter_map(|id| {
            let v = crate::rules::classes::class(&id).ok()?;
            let f = crate::rules::classes::design_figures(&v, None);
            Some(Mark {
                within: within(f.loa_m, f.beam_m),
                id,
                custom: false,
                loa_m: f.loa_m,
                beam_m: f.beam_m,
            })
        })
        .collect();
    if let Some(c) = custom {
        marks.push(Mark {
            id: c.label(),
            custom: true,
            loa_m: c.loa_m,
            beam_m: c.beam_m,
            within: within(c.loa_m, c.beam_m),
        });
    }
    Envelope {
        settings,
        length_m,
        sizes,
        largest,
        per_km,
        marks,
    }
}

/// The whole sweep, one size after another.
pub fn envelope(
    reference: &[Pt],
    water: &[Polygon],
    settings: Settings,
    custom: Option<&CustomVessel>,
) -> Result<Envelope, PyError> {
    if reference.len() < 2 {
        return Err(PyError::value("the reference line needs at least two points."));
    }
    let sizes = settings
        .sizes
        .iter()
        .map(|&s| size_result(reference, water, &settings, s))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(combine(reference, settings, sizes, custom))
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    /// A channel `width` wide along `centre` (points with left normals). Returns the water.
    fn water_along(c: &[(Pt, Pt)], width: f64) -> Polygon {
        let h = width / 2.0;
        let mut ring: Vec<Pt> = c
            .iter()
            .map(|(p, n)| [p[0] - h * n[0], p[1] - h * n[1]])
            .collect();
        ring.extend(c.iter().rev().map(|(p, n)| [p[0] + h * n[0], p[1] + h * n[1]]));
        ring.push(ring[0]);
        Polygon {
            exterior: ring,
            holes: vec![],
        }
    }

    fn straight(len: f64) -> Vec<(Pt, Pt)> {
        (0..=(len / 10.0) as usize)
            .map(|i| ([10.0 * i as f64, 0.0], [0.0, 1.0]))
            .collect()
    }

    /// Straight 1500 m, a 90° left bend of radius `rc`, straight 1500 m.
    fn bend(rc: f64) -> Vec<(Pt, Pt)> {
        let mut c: Vec<(Pt, Pt)> = (0..150)
            .map(|i| ([-1500.0 + 10.0 * i as f64, 0.0], [0.0, 1.0]))
            .collect();
        let n = (rc * core::f64::consts::FRAC_PI_2 / 5.0).ceil() as usize;
        for i in 0..n {
            let t =
                -core::f64::consts::FRAC_PI_2 + core::f64::consts::FRAC_PI_2 * i as f64 / n as f64;
            let (s, co) = (libm::sin(t), libm::cos(t));
            c.push(([rc * co, rc + rc * s], [-co, -s]));
        }
        for i in 0..=150 {
            c.push(([rc, rc + 10.0 * i as f64], [-1.0, 0.0]));
        }
        c
    }

    /// The centre of the channel, 20 m short of the water's ends (as the reference line is cut
    /// back: its ends lie inside the water, not on its end edges).
    fn line(c: &[(Pt, Pt)]) -> Vec<Pt> {
        c[2..c.len() - 2].iter().map(|(p, _)| *p).collect()
    }

    /// The bottom width and R_min of a size under the default settings.
    fn shape(i: usize) -> (f64, f64) {
        let s = default_sizes()[i];
        let r = size_result(
            &line(&straight(100.0)),
            &[water_along(&straight(100.0), 1000.0)],
            &Settings::default(),
            s,
        )
        .unwrap();
        (r.bottom_width_straight_m, r.r_min_m)
    }

    #[test]
    fn a_channel_built_to_pass_30x6_and_fail_40x8_answers_30x6() {
        let (w30, _) = shape(2);
        let (w40, _) = shape(3);
        assert!(w30 < w40, "{w30} {w40}");
        let c = straight(3000.0);
        let e = envelope(
            &line(&c),
            &[water_along(&c, (w30 + w40) / 2.0)],
            Settings::default(),
            None,
        )
        .unwrap();
        assert_eq!(e.largest, Some(2), "{:?}", e.sizes.iter().map(|s| s.passes).collect::<Vec<_>>());
        assert!(e.per_km.iter().all(|k| k.largest == Some(2)));
        assert!(!e.sizes[3].stretches.is_empty());
        assert!(e.sizes[3].stretches.iter().all(|s| s.kind.as_str() == "width"));
    }

    #[test]
    fn a_kilometre_with_a_tight_bend_gets_a_smaller_limit_than_its_neighbours() {
        // Water just wide enough for the largest size, so a fit cannot ease the bend much; the
        // bend's radius sits between the R_min of the 20 m and the 30 m size.
        let (w50, _) = shape(4);
        let (_, r20) = shape(1);
        let (_, r30) = shape(2);
        let rc = (r20 + r30) / 2.0;
        let c = bend(rc);
        let e = envelope(&line(&c), &[water_along(&c, w50 + 4.0)], Settings::default(), None)
            .unwrap();
        let first = e.per_km[0].largest;
        let last = e.per_km[e.per_km.len() - 1].largest;
        let (bend_from, bend_to) = (1500.0, 1500.0 + rc * core::f64::consts::FRAC_PI_2);
        let at_bend = e
            .per_km
            .iter()
            .filter(|k| k.from_m < bend_to && k.to_m > bend_from)
            .map(|k| k.largest)
            .min()
            .unwrap();
        assert_eq!(first, Some(4));
        assert_eq!(last, Some(4));
        assert!(at_bend < first, "bend {at_bend:?}, ends {first:?}");
        assert!(e.largest < Some(4));
    }

    #[test]
    fn more_clearance_or_a_stricter_rule_never_raises_the_answer() {
        let (w30, _) = shape(2);
        let c = straight(2000.0);
        let water = [water_along(&c, w30 + 6.0)];
        let run = |s: Settings| envelope(&line(&c), &water, s, None).unwrap().largest;
        let base = run(Settings::default());
        for cl in [2.0, 5.0, 10.0] {
            let more = run(Settings {
                clearance_m: cl,
                ..Settings::default()
            });
            assert!(more <= base, "clearance {cl}: {more:?} > {base:?}");
        }
        for rs in ["PIANC-B", "PIANC-A"] {
            let strict = run(Settings {
                rule_set: rs.into(),
                ..Settings::default()
            });
            assert!(strict <= base, "{rs}: {strict:?} > {base:?}");
        }
    }

    #[test]
    fn the_catalogue_is_marked_against_the_largest_size() {
        let c = straight(1000.0);
        let e = envelope(&line(&c), &[water_along(&c, 2000.0)], Settings::default(), None)
            .unwrap();
        assert_eq!(e.largest, Some(4));
        assert_eq!(e.marks.len(), 12);
        for m in &e.marks {
            assert_eq!(m.within, m.loa_m <= 50.0 && m.beam_m <= 10.0, "{}", m.id);
        }
    }

    #[test]
    fn sizes_are_read_from_a_list() {
        let s = Size::parse_list("10x2, 25X5,40×8").unwrap();
        assert_eq!(s[1], Size { loa_m: 25.0, beam_m: 5.0 });
        assert_eq!(s[2].label(), "40x8");
        assert!(Size::parse_list("10x").is_err());
        assert!(Size::parse_list("10x-2").is_err());
    }
}

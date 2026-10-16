//! The search-class check (ADR 0005): how a case of a search entry point is judged.
//!
//! A search makes discrete choices. Where two candidates cost almost the same, a change in the
//! order of floating-point operations can pick the other one. So:
//!
//! - A case the generator did NOT record as a near tie is compared exactly, like any other case:
//!   the same path as the frozen Python.
//! - A case recorded as a near tie (`search.near_tie` in the case file, set when the Reference
//!   Result was printed, never after a failure) is compared on its objective, within
//!   `search_objective_rel`, and on its constraints, which must still match the Reference Result.
//!   The exact path is not compared.
//!
//! Each search function says what its objective and its constraints are, in [`view`].
//! The rolling circle's objective is the length of the line it returns; its constraints are
//! `radius_ok`, the measured minimum radius and the chord it was read on. Deleted with the
//! Reference Results (ticket 37).

use crate::parity::{Tolerance, compare};
use serde_json::{Value, json};

/// How a search result is judged when its case is a near tie.
#[derive(Debug, Clone, PartialEq)]
struct View {
    /// The quantity the search minimises or reports, if the function has one.
    objective: Option<f64>,
    /// What must still match the Reference Result, whatever path was taken.
    constraints: Value,
}

/// How a case was judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judged {
    /// Compared exactly (the usual case, and every search case that is not a near tie).
    Exact,
    /// A recorded near tie: judged on objective and constraints, not on the path.
    NearTie,
}

fn float(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn path_length(coords: &Value) -> Option<f64> {
    let pts: Vec<[f64; 2]> = coords
        .as_array()?
        .iter()
        .map(|p| Some([float(p.get(0)?)?, float(p.get(1)?)?]))
        .collect::<Option<_>>()?;
    Some(
        pts.windows(2)
            .map(|w| {
                let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                (dx * dx + dy * dy).sqrt()
            })
            .sum(),
    )
}

/// The objective and constraints of one result, or `None` if the function is not a search or the
/// result is a refusal.
fn view(entry_point: &str, input: &Value, output: &Value) -> Option<View> {
    let function = input.get("fn").and_then(Value::as_str).unwrap_or("");
    let ok = output.get("ok")?;
    match (entry_point, function) {
        ("rolling_circle", "rolling_circle") => {
            let r = ok.get("result")?;
            Some(View {
                objective: Some(path_length(r.get("centreline")?.get("coords")?)?),
                constraints: json!({
                    "radius_ok": r.get("radius_ok")?,
                    "measured_min_radius_m": r.get("measured_min_radius_m")?,
                    "radius_check_chord_m": r.get("radius_check_chord_m")?,
                }),
            })
        }
        ("rolling_circle", "find_forward_circle_contact") => Some(View {
            objective: ok.get("contact_chainage").and_then(float),
            constraints: json!({ "found": !ok.is_null() }),
        }),
        // The DP's objective is its total path cost; it has no constraint beyond the path itself.
        ("route", "dp_optimise") => Some(View {
            objective: ok.get("cost").and_then(float),
            constraints: json!({}),
        }),
        // The corridor solve: the saving it reports, and whether its line clears the target
        // radius (the reading tolerance below `r_min`, as `solve_at_corridor` enforces).
        ("route", "solve_at_corridor") => {
            let r_min = input.get("args")?.get("r_min").and_then(float)?;
            let min_radius = ok.get("min_radius").and_then(float)?;
            Some(View {
                objective: ok.get("saving_pct").and_then(float),
                constraints: json!({ "clears_target": min_radius >= 0.98 * r_min - 1e-6 }),
            })
        }
        // The sweep's winner: the saving it reports; `radius_ok`, the measured radius and the
        // chord it was read on must still match.
        ("route", "optimise_route") => Some(View {
            objective: ok.get("approx_saving_pct").and_then(float),
            constraints: json!({
                "radius_ok": ok.get("radius_ok")?,
                "measured_min_radius_m": ok.get("measured_min_radius_m")?,
                "radius_check_chord_m": ok.get("radius_check_chord_m")?,
            }),
        }),
        _ => None,
    }
}

/// Judge one case. `search` is the case's `search` record from the case file, if it has one.
/// On failure returns every difference found.
pub fn judge(
    entry_point: &str,
    input: &Value,
    search: Option<&Value>,
    expected: &Value,
    actual: &Value,
    tol: Tolerance,
) -> Result<Judged, Vec<String>> {
    let near_tie = search
        .and_then(|s| s.get("near_tie"))
        .and_then(Value::as_bool)
        == Some(true);
    let views = view(entry_point, input, expected).zip(view(entry_point, input, actual));
    let (Some((e, a)), true) = (views, near_tie) else {
        return compare(expected, actual, tol).map(|()| Judged::Exact);
    };
    let mut diffs = Vec::new();
    match (e.objective, a.objective) {
        (Some(x), Some(y)) => {
            if !(x == y
                || (x - y).abs() <= (tol.search_objective_rel * x.abs().max(y.abs())).max(tol.abs))
            {
                diffs.push(format!("objective: expected {x}, got {y}"));
            }
        }
        (None, None) => {}
        (x, y) => diffs.push(format!("objective: expected {x:?}, got {y:?}")),
    }
    if let Err(d) = compare(&e.constraints, &a.constraints, tol) {
        diffs.extend(d.into_iter().map(|m| format!("constraint {m}")));
    }
    if diffs.is_empty() {
        Ok(Judged::NearTie)
    } else {
        Err(diffs)
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    const TOL: Tolerance = Tolerance {
        rel: 1e-9,
        abs: 1e-9,
        search_objective_rel: 1e-9,
        polygon_area_rel: 1e-9,
        polygon_vertex_m: 1e-6,
    };

    fn run(coords: Value, ok: bool, measured: f64) -> Value {
        json!({"ok": {"result": {
            "centreline": {"geom": "LineString", "coords": coords},
            "min_radius_m": 300.0, "circles": [],
            "measured_min_radius_m": measured, "radius_check_chord_m": 100.0, "radius_ok": ok,
        }, "n_arcs": 0}})
    }

    fn case() -> Value {
        json!({"fn": "rolling_circle", "args": {}})
    }

    fn tie(flag: bool) -> Value {
        json!({"what": "contact chainage, m", "near_tie": flag})
    }

    // Two paths of the same length (one detours up, one down), different vertices.
    fn path_a() -> Value {
        json!([[0.0, 0.0], [30.0, 40.0], [60.0, 0.0]])
    }
    fn path_b() -> Value {
        json!([[0.0, 0.0], [30.0, -40.0], [60.0, 0.0]])
    }

    #[test]
    fn a_recorded_near_tie_passes_on_another_path_with_the_same_objective_and_constraints() {
        let (e, a) = (run(path_a(), true, 299.0), run(path_b(), true, 299.0));
        let t = tie(true);
        assert_eq!(
            judge("rolling_circle", &case(), Some(&t), &e, &a, TOL),
            Ok(Judged::NearTie)
        );
    }

    #[test]
    fn a_path_that_differs_without_a_recorded_near_tie_fails() {
        let (e, a) = (run(path_a(), true, 299.0), run(path_b(), true, 299.0));
        for t in [Some(tie(false)), None] {
            let diffs = judge("rolling_circle", &case(), t.as_ref(), &e, &a, TOL).unwrap_err();
            assert!(diffs[0].contains("$.ok.result.centreline"), "{diffs:?}");
        }
    }

    #[test]
    fn a_near_tie_with_another_objective_fails() {
        let longer = json!([[0.0, 0.0], [30.0, 40.0], [60.0, 0.001]]);
        let t = tie(true);
        let diffs = judge(
            "rolling_circle",
            &case(),
            Some(&t),
            &run(path_a(), true, 299.0),
            &run(longer, true, 299.0),
            TOL,
        )
        .unwrap_err();
        assert!(diffs[0].starts_with("objective:"), "{diffs:?}");
    }

    #[test]
    fn a_near_tie_must_still_meet_every_constraint_of_the_reference() {
        let t = tie(true);
        let e = run(path_a(), true, 299.0);
        for bad in [run(path_b(), false, 299.0), run(path_b(), true, 290.0)] {
            let diffs = judge("rolling_circle", &case(), Some(&t), &e, &bad, TOL).unwrap_err();
            assert!(
                diffs.iter().any(|d| d.starts_with("constraint")),
                "{diffs:?}"
            );
        }
    }

    #[test]
    fn a_refusal_is_compared_exactly_even_in_a_near_tie_case() {
        let t = tie(true);
        let err = json!({"error": {"type": "ValueError", "message": "step must be positive."}});
        assert_eq!(
            judge("rolling_circle", &case(), Some(&t), &err, &err, TOL),
            Ok(Judged::Exact)
        );
        assert!(
            judge(
                "rolling_circle",
                &case(),
                Some(&t),
                &err,
                &run(path_a(), true, 1.0),
                TOL
            )
            .is_err()
        );
    }

    fn route_out(saving: f64, ok: bool, measured: f64, chord: f64) -> Value {
        json!({"ok": {"line": {"geom": "LineString", "coords": []}, "offsets": [0.0],
            "approx_saving_pct": saving, "radius_ok": ok,
            "measured_min_radius_m": measured, "radius_check_chord_m": chord}})
    }

    #[test]
    fn a_route_near_tie_is_judged_on_saving_and_the_radius_reading() {
        let input = json!({"fn": "optimise_route", "args": {}});
        let t = tie(true);
        let e = route_out(12.5, true, 600.0, 400.0);
        let mut other_path = route_out(12.5, true, 600.0, 400.0);
        other_path["ok"]["offsets"] = json!([20.0]); // another path, same saving and reading
        assert_eq!(
            judge("route", &input, Some(&t), &e, &other_path, TOL),
            Ok(Judged::NearTie)
        );
        for bad in [
            route_out(12.6, true, 600.0, 400.0),
            route_out(12.5, false, 600.0, 400.0),
            route_out(12.5, true, 590.0, 400.0),
            route_out(12.5, true, 600.0, 200.0),
        ] {
            assert!(judge("route", &input, Some(&t), &e, &bad, TOL).is_err());
        }
        // Not recorded as a near tie: the path is compared.
        assert!(judge("route", &input, Some(&tie(false)), &e, &other_path, TOL).is_err());
    }

    #[test]
    fn a_dp_near_tie_passes_on_the_same_cost_and_fails_on_another() {
        let input = json!({"fn": "dp_optimise", "args": {}});
        let t = tie(true);
        let e = json!({"ok": {"path": [1, 2], "cost": 100.0}});
        let same_cost = json!({"ok": {"path": [2, 1], "cost": 100.0}});
        let dearer = json!({"ok": {"path": [2, 1], "cost": 100.001}});
        assert_eq!(
            judge("route", &input, Some(&t), &e, &same_cost, TOL),
            Ok(Judged::NearTie)
        );
        assert!(judge("route", &input, Some(&t), &e, &dearer, TOL).is_err());
    }
}

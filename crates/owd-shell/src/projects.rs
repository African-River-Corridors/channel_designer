//! Where a study from a river is saved (#322): its own folder in the projects folder, named
//! `<YYYY-MM-DD> <River> km <start>-<end>`, holding the document and every download. The names are
//! made here; the desktop shell does the file work.

use crate::state::AppState;
use owd_reach::geo::haversine_m;

/// The default projects folder, under the user's home: `~/Documents/Open Waterway Designer`.
pub const DEFAULT_PROJECTS: &str = "Documents/Open Waterway Designer";

/// A name that is safe as a file or folder name on Windows, macOS and Linux: no `/ \ : * ? " < > |`
/// or control characters, no leading or trailing dots and spaces, single spaces, at most 120
/// characters. Empty becomes "Study".
pub fn safe_name(s: &str) -> String {
    let kept: String = s
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut out = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    out = out.trim_matches(|c| c == '.' || c == ' ').to_string();
    if out.chars().count() > 120 {
        out = out
            .chars()
            .take(120)
            .collect::<String>()
            .trim_end()
            .to_string();
    }
    if out.is_empty() {
        "Study".to_string()
    } else {
        out
    }
}

/// `2026-10-22 Rhine km 0-1006`: the date, the river and the reach's kilometres, rounded.
pub fn study_folder_name(date: &str, river: &str, from_km: f64, to_km: f64) -> String {
    let (a, b) = if from_km <= to_km {
        (from_km, to_km)
    } else {
        (to_km, from_km)
    };
    safe_name(&format!(
        "{date} {river} km {:.0}-{:.0}",
        a.max(0.0),
        b.max(0.0)
    ))
}

/// The distance along `line` from its first point to the vertex nearest `p`, in kilometres.
fn km_along(line: &[[f64; 2]], p: [f64; 2]) -> f64 {
    let mut run = 0.0;
    let mut best = (f64::INFINITY, 0.0);
    for (i, q) in line.iter().enumerate() {
        if i > 0 {
            run += haversine_m(line[i - 1], *q);
        }
        let d = haversine_m(*q, p);
        if d < best.0 {
            best = (d, run);
        }
    }
    best.1 / 1000.0
}

impl AppState {
    /// The folder name of the study this session holds, for its date: the river and the reach's
    /// kilometres from the river's mouth when it was chosen from the search, else from the route's
    /// start. `None` with no reach planned.
    pub fn study_folder_name(&self, date: &str) -> Option<String> {
        let view = self.reach_view();
        let route: Vec<[f64; 2]> = serde_json::from_value(view["route"].clone()).ok()?;
        let length_km = view["length_m"].as_f64().unwrap_or(0.0) / 1000.0;
        let doc = self.session().document();
        let river = view["river"]["name"]
            .as_str()
            .or(doc["reach"]["river"].as_str())
            .or(doc["project"]["title"].as_str())
            .unwrap_or("Study")
            .to_string();
        let (a, b) = match serde_json::from_value::<Vec<[f64; 2]>>(view["river"]["line"].clone()) {
            Ok(line) if !line.is_empty() && !route.is_empty() => (
                km_along(&line, route[0]),
                km_along(&line, route[route.len() - 1]),
            ),
            _ => (0.0, length_km),
        };
        Some(study_folder_name(date, &river, a, b))
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn names_follow_the_pattern_and_drop_unsafe_characters() {
        assert_eq!(
            study_folder_name("2026-10-22", "Rhine", 0.0, 1005.8),
            "2026-10-22 Rhine km 0-1006"
        );
        assert_eq!(
            study_folder_name("2026-10-22", "Ankobra River", 120.4, 18.0),
            "2026-10-22 Ankobra River km 18-120"
        );
        assert_eq!(
            safe_name(" a/b\\c:d*e?f\"g<h>i|j\u{7}k. "),
            "a b c d e f g h i j k"
        );
        assert_eq!(safe_name("..."), "Study");
        assert_eq!(safe_name(&"x".repeat(200)).len(), 120);
        assert_eq!(
            safe_name("Côte d'Ivoire  —  Sassandra"),
            "Côte d'Ivoire — Sassandra"
        );
    }

    #[test]
    fn kilometres_count_from_the_start_of_the_river_line() {
        // About 1.1 km between points along the equator.
        let line = [[0.0, 0.0], [0.01, 0.0], [0.02, 0.0], [0.03, 0.0]];
        assert!(km_along(&line, [0.0, 0.0]).abs() < 1e-9);
        let k = km_along(&line, [0.0201, 0.0001]);
        assert!((k - 2.224).abs() < 0.01, "{k}");
    }
}

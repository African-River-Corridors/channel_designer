//! Where a study from a river is saved (#322): its own folder in the projects folder, named
//! `<YYYY-MM-DD HHMM> <River>` by the minute the study started (the reach's chainage can change),
//! holding the document and every download. The names are made here; the desktop shell does the
//! file work and reads the clock.

use crate::state::AppState;

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

/// `2026-10-22 1435 Rhine`: the minute the study started (`YYYY-MM-DD HHMM`, local time) and the
/// river.
pub fn study_folder_name(started: &str, river: &str) -> String {
    safe_name(&format!("{started} {river}"))
}

impl AppState {
    /// The river of the study this session holds, for its folder name: the river chosen from the
    /// search, else the river the reach names, else the study's title.
    pub fn study_river(&self) -> String {
        let view = self.reach_view();
        let doc = self.session().document();
        view["river"]["name"]
            .as_str()
            .or(doc["reach"]["river"].as_str())
            .or(doc["project"]["title"].as_str())
            .unwrap_or("Study")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn names_follow_the_pattern_and_drop_unsafe_characters() {
        assert_eq!(
            study_folder_name("2026-10-22 1435", "Rhine"),
            "2026-10-22 1435 Rhine"
        );
        assert_eq!(
            study_folder_name("2026-10-22 0905", "Ankobra River"),
            "2026-10-22 0905 Ankobra River"
        );
        assert_eq!(
            study_folder_name("2026-10-22 1435", "a/b\\c:d*e?f\"g<h>i|j"),
            "2026-10-22 1435 a b c d e f g h i j"
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
}

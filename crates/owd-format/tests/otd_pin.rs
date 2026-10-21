//! The `otd` dependency is pinned by git tag of its own repository, never by a path or a branch
//! (spec 0004, ADR 0009), and the crate that uses it takes it from the workspace.
//!
//! Label: property

#[test]
fn otd_is_pinned_by_tag_in_the_workspace_and_used_from_it() {
    let root = include_str!("../../../Cargo.toml");
    let line = root
        .lines()
        .find(|l| l.trim_start().starts_with("otd ="))
        .expect("the workspace has an `otd` dependency");
    assert!(
        line.contains("git = \"https://github.com/African-River-Corridors/open-technical-drawer\""),
        "{line}"
    );
    assert!(line.contains("tag = \"v"), "pin by tag: {line}");
    for banned in ["path", "branch", "rev"] {
        assert!(
            !line.contains(&format!("{banned} =")),
            "otd must not use `{banned}`: {line}"
        );
    }
    let own = include_str!("../Cargo.toml");
    assert!(
        own.contains("otd.workspace = true"),
        "owd-format takes otd from the workspace"
    );
}

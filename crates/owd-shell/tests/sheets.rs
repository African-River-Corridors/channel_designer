//! The app's Sheets view (spec 0004, ticket 8) through the shell: the sheets are those `owd draw`
//! makes, and each exported PDF has the bytes `owd draw` writes for the same design and style.
//!
//! Label: property

use owd_shell::state::AppState;
use std::path::{Path, PathBuf};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web/river-example.owd")
}

fn opened() -> AppState {
    let text = std::fs::read_to_string(example()).unwrap();
    let dem = std::fs::read(example().with_file_name("river-dem.tif")).unwrap();
    AppState::open(&example(), &text, Some(&dem)).expect("the example opens with its DEM")
}

#[test]
fn the_view_gets_every_sheet_with_its_svg() {
    let state = opened();
    let option = "rolling-circle";
    let pages = state.sheets_job(option).pages().unwrap();
    let pages = pages.as_array().unwrap();
    // the same sheets as `owd draw` makes
    let text = std::fs::read_to_string(example()).unwrap();
    let dem = std::fs::read(example().with_file_name("river-dem.tif")).unwrap();
    let cli =
        owd_format::drawing::sheets_from_text(&text, Some(&dem), &Default::default()).unwrap();
    let numbers: Vec<&str> = pages
        .iter()
        .map(|p| p["number"].as_str().unwrap())
        .collect();
    let want: Vec<&str> = cli.iter().map(|s| s.number.as_str()).collect();
    assert_eq!(numbers, want);
    for p in pages {
        assert!(
            p["svg"].as_str().unwrap().starts_with("<?xml"),
            "{}",
            p["number"]
        );
        assert!(!p["title"].as_str().unwrap().is_empty());
    }
    assert!(
        numbers.iter().any(|n| n.contains("-PL-")),
        "the plan sheets too"
    );
}

#[test]
fn exported_pdfs_are_byte_identical_to_owd_draw() {
    let state = opened();
    let pdfs = state.sheets_job("rolling-circle").pdfs().unwrap();
    let dir = std::env::temp_dir().join(format!("owd-sheets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // `owd draw FILE --option rolling-circle --out DIR`, through the command itself
    owd_cli::commands::draw(
        example().to_str().unwrap(),
        owd_cli::commands::DrawFlags {
            out: Some(dir.to_string_lossy().into_owned()),
            documents_only: false,
            option: Some("rolling-circle".into()),
            sheets: None,
            revision: None,
            date: None,
            style: None,
            section_spacing: None,
            sheet_size: None,
            scale: None,
            vertical_exaggeration: None,
            orthophoto: None,
            imagery: None,
            imagery_cache: None,
        },
        false,
    )
    .map_err(|f| f.message)
    .unwrap();
    assert!(!pdfs.is_empty());
    for (name, bytes) in &pdfs {
        let cli = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(cli == *bytes, "{name} differs from owd draw's");
    }
    let written = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "pdf")
        })
        .count();
    assert_eq!(written, pdfs.len(), "every sheet, no more");
    let _ = std::fs::remove_dir_all(&dir);
}

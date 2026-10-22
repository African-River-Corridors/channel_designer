//! `owd qa-overlay` (spec 0013): draw a document's banks, centreline and low-confidence stretches
//! over the Sentinel-2 true-colour image of the scenes its banks came from, as a PNG and a KMZ, so a
//! person can check a reach by eye.

use crate::commands::{Result, input_error, open, say};
use owd_reach::overlay::{OverlayRequest, render_document};
use owd_reach::sentinel2::SEARCH_ENDPOINT;
use serde_json::json;

/// The arguments of `owd qa-overlay`.
pub struct OverlayFlags {
    pub doc: String,
    pub png: String,
    pub kmz: Option<String>,
    pub from_km: Option<f64>,
    pub to_km: Option<f64>,
    /// Scenes to draw on, by id, instead of the ones the document names.
    pub scene: Vec<String>,
    pub stac: Option<String>,
    pub json: bool,
}

pub fn qa_overlay(flags: OverlayFlags) -> Result {
    qa_overlay_with(flags, &crate::reach_fetch::HttpFetch)
}

/// `qa_overlay` with the fetcher given, so a test can serve recorded answers.
pub fn qa_overlay_with(flags: OverlayFlags, fetch: &dyn owd_reach::Fetch) -> Result {
    let session = open(&flags.doc)?;
    let req = OverlayRequest {
        from_m: flags.from_km.map(|k| k * 1000.0),
        to_m: flags.to_km.map(|k| k * 1000.0),
        scenes: flags.scene.clone(),
    };
    let r = render_document(
        fetch,
        session.document(),
        flags.stac.as_deref().unwrap_or(SEARCH_ENDPOINT),
        &req,
    )
    .map_err(|e| input_error(e.to_string()))?;
    std::fs::write(&flags.png, &r.png)
        .map_err(|e| input_error(format!("cannot write {}: {e}", flags.png)))?;
    if let Some(path) = &flags.kmz {
        std::fs::write(path, &r.kmz)
            .map_err(|e| input_error(format!("cannot write {path}: {e}")))?;
    }
    let human = format!(
        "wrote {}{}: {} x {} px over km {:.2} to {:.2}, on {}\n{}",
        flags.png,
        flags
            .kmz
            .as_ref()
            .map_or(String::new(), |k| format!(" and {k}")),
        r.width,
        r.height,
        r.from_m / 1000.0,
        r.to_m / 1000.0,
        r.scenes.join(", "),
        r.notices.join("\n")
    );
    say(
        false,
        flags.json,
        &human,
        json!({"ok": true, "png": flags.png, "kmz": flags.kmz, "width_px": r.width, "height_px": r.height,
               "from_m": r.from_m, "to_m": r.to_m, "scenes": r.scenes, "flags": r.flags, "notices": r.notices}),
    );
    Ok(())
}

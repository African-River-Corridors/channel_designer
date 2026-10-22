//! `owd qa-overlay` (spec 0013): draw a document's banks, centreline and low-confidence stretches
//! over the Sentinel-2 true-colour image of the scenes its banks came from, as a PNG and a KMZ, so a
//! person can check a reach by eye.

use crate::commands::{Result, input_error, open, say};
use owd_reach::overlay::{Shot, kmz, lines_from_document, scene_ids, true_colour};
use owd_reach::sentinel2::{SEARCH_ENDPOINT, item_url, parse_item, read_blue, read_imagery};
use serde_json::json;

/// The largest picture, in pixels each way (30 km at 10 m): a longer reach is drawn a part at a
/// time with `--from-km` and `--to-km`.
pub const MAX_SIDE_PX: f64 = 3_000.0;

/// The margin around the lines, in metres.
const MARGIN_M: f64 = 300.0;

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
    let doc = session.document();
    let (utm, all) = lines_from_document(doc).map_err(|e| input_error(e.to_string()))?;
    let length = owd_format::design::midline::arc(&all.centre)
        .last()
        .copied()
        .unwrap_or(0.0);
    let (a, b) = (
        flags.from_km.map_or(0.0, |k| k * 1000.0),
        flags.to_km.map_or(length, |k| k * 1000.0),
    );
    if a >= b || a.is_nan() || b.is_nan() {
        return Err(input_error("--from-km must be less than --to-km"));
    }
    let lines = all.crop(a, b);
    let (lo, hi) = lines
        .bounds()
        .ok_or_else(|| input_error("nothing to draw between those chainages"))?;
    let (min, max) = (
        [lo[0] - MARGIN_M, lo[1] - MARGIN_M],
        [hi[0] + MARGIN_M, hi[1] + MARGIN_M],
    );
    if (max[0] - min[0]) / 10.0 > MAX_SIDE_PX || (max[1] - min[1]) / 10.0 > MAX_SIDE_PX {
        return Err(input_error(format!(
            "the stretch is more than {:.0} km across: draw part of it with --from-km and --to-km \
             (the centreline is {:.1} km long)",
            MAX_SIDE_PX / 100.0,
            length / 1000.0
        )));
    }
    let ids = if flags.scene.is_empty() {
        scene_ids(doc)
    } else {
        flags.scene.clone()
    };
    if ids.is_empty() {
        return Err(input_error(
            "the document names no Sentinel-2 scene: give one with --scene",
        ));
    }
    let endpoint = flags.stac.as_deref().unwrap_or(SEARCH_ENDPOINT);
    let fail = |e: owd_reach::ReachError| input_error(e.to_string());
    let mut windows = Vec::new();
    let mut used = Vec::new();
    for id in &ids {
        let scene = parse_item(
            &fetch
                .get(&item_url(endpoint, id))
                .map_err(|e| input_error(e.to_string()))?,
        )
        .map_err(fail)?;
        if scene.epsg.is_some_and(|e| e != utm.epsg()) {
            continue; // another UTM zone: its grid does not line up with the study's
        }
        let im = read_imagery(fetch, &scene, min, max).map_err(fail)?;
        let blue = read_blue(fetch, &scene, &im).map_err(fail)?;
        used.push(scene);
        windows.push((im, blue));
    }
    if windows.is_empty() {
        return Err(input_error(format!(
            "none of the scenes {ids:?} is in the study's UTM zone {}",
            utm.zone
        )));
    }
    let shots: Vec<Shot> = windows
        .iter()
        .map(|(im, blue)| Shot { imagery: im, blue })
        .collect();
    // Up to 4 picture pixels per 10 m, so a small window is not a thumbnail.
    let raw = true_colour(&shots).map_err(fail)?;
    let k = (1_600 / raw.width.max(raw.map_rows)).clamp(1, 4);
    let mut map = raw.upscale(k);
    map.draw(&lines);
    let s2 = owd_reach::source("sentinel2-l2a").expect("sentinel2-l2a is in the registry");
    let mut years: Vec<String> = used
        .iter()
        .map(|s| s.date.chars().take(4).collect())
        .collect();
    years.sort();
    years.dedup();
    // The image's notices, then those of the bank lines (the water mask's among them).
    let mut notices: Vec<String> = years.iter().map(|y| s2.notice_for(y)).collect();
    let banks = doc["provenance"]["left_bank"]["notice"]
        .as_str()
        .unwrap_or("");
    for n in banks.split("; ").filter(|n| !n.is_empty()) {
        if !notices.iter().any(|o| o == n) && !n.contains("Copernicus Sentinel") {
            notices.push(n.to_string());
        }
    }
    let mut text: Vec<String> = vec![format!(
        "{} km {:.2} to {:.2}; scenes: {}",
        doc["project"]["title"].as_str().unwrap_or(""),
        a / 1000.0,
        b / 1000.0,
        used.iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )];
    text.extend(notices.iter().cloned());
    let png = map.with_legend(&text).png().map_err(fail)?;
    std::fs::write(&flags.png, &png)
        .map_err(|e| input_error(format!("cannot write {}: {e}", flags.png)))?;
    if let Some(path) = &flags.kmz {
        let name = format!(
            "{} km {:.2} to {:.2}",
            doc["project"]["title"].as_str().unwrap_or("Reach"),
            a / 1000.0,
            b / 1000.0
        );
        let bytes = kmz(&map, &lines, &name, &notices).map_err(fail)?;
        std::fs::write(path, bytes)
            .map_err(|e| input_error(format!("cannot write {path}: {e}")))?;
    }
    let human = format!(
        "wrote {}{}: {} x {} px over km {:.2} to {:.2}, on {}\n{}",
        flags.png,
        flags
            .kmz
            .as_ref()
            .map_or(String::new(), |k| format!(" and {k}")),
        map.width,
        map.height,
        a / 1000.0,
        b / 1000.0,
        used.iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        notices.join("\n")
    );
    say(
        false,
        flags.json,
        &human,
        json!({"ok": true, "png": flags.png, "kmz": flags.kmz, "width_px": map.width, "height_px": map.height,
               "from_m": a, "to_m": b, "scenes": used.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
               "flags": lines.flags.len(), "notices": notices}),
    );
    Ok(())
}

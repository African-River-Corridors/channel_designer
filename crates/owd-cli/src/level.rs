//! `owd water-level`: the default low-water level from SWORD and SWOT, with the DEM where they give
//! none (spec 0012, T6).

use crate::commands::{
    Result, destination, input_error, open_with_dem, read_dem, say, today, write,
};
use owd_format::design::DemCheck;
use owd_format::design::dem_source::COPERNICUS_NOTICE;
use owd_format::design::provenance::Mark;
use owd_reach::level::{
    Datum, HYDROCRON_ENDPOINT, LevelParams, SWOT_SCIENCE_START, Source, level_from_sources,
    record_level,
};
use serde_json::json;

/// The arguments of `owd water-level`.
pub struct LevelFlags {
    pub doc: String,
    pub percentile: Option<f64>,
    pub min_swot_width: Option<f64>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub dem_datum: Option<String>,
    pub output: Option<String>,
    pub json: bool,
}

/// Where the sources are: the real ones, or recorded ones in a test.
pub struct Endpoints<'a> {
    pub sword_record: &'a str,
    pub hydrocron: &'a str,
}

pub fn water_level(flags: LevelFlags) -> Result {
    water_level_with(
        flags,
        &crate::reach_fetch::HttpFetch,
        &Endpoints {
            sword_record: owd_reach::sword::RECORD_API,
            hydrocron: HYDROCRON_ENDPOINT,
        },
    )
}

/// `water_level` with the fetcher and the endpoints given, so a test can serve recorded answers.
pub fn water_level_with(flags: LevelFlags, fetch: &dyn owd_reach::Fetch, at: &Endpoints) -> Result {
    let mut params = LevelParams::default();
    if let Some(p) = flags.percentile {
        if !(0.0..=100.0).contains(&p) {
            return Err(input_error("--percentile must be from 0 to 100"));
        }
        params.percentile = p;
    }
    if let Some(w) = flags.min_swot_width {
        if !(w >= 0.0 && w.is_finite()) {
            return Err(input_error("--min-swot-width must be 0 or more metres"));
        }
        params.min_swot_width_m = w;
    }
    let (mut session, bytes) = open_with_dem(&flags.doc)?;
    let (centre, utm) = session
        .water_reference()
        .map_err(|e| input_error(e.to_string()))?;
    let copernicus = !session.dem_source_tiles().is_empty();
    let datum = match flags.dem_datum.as_deref() {
        None | Some("egm2008") => Datum::Egm2008,
        Some("ellipsoid") => Datum::Ellipsoid,
        Some(o) => {
            return Err(input_error(format!(
                "--dem-datum is `egm2008` or `ellipsoid`, not `{o}`"
            )));
        }
    };
    if copernicus && datum != Datum::Egm2008 {
        return Err(input_error(
            "the document's DEM is Copernicus GLO-30, which is in EGM2008 heights: leave out --dem-datum",
        ));
    }
    // The DEM only when it is the file whose hash was recorded.
    let dem = match (session.dem_check(bytes.as_deref()), &bytes) {
        (DemCheck::Matches, Some(b)) => Some(read_dem(b, session.dem_path().unwrap_or("DEM"))?),
        _ => None,
    };
    let study = utm.epsg();
    let sampler = dem.as_ref().and_then(|d| {
        let f: Box<dyn Fn([f64; 2]) -> f64> = match d.epsg {
            Some(4326) => Box::new(move |p: [f64; 2]| d.sample_bilinear(p[0], p[1])),
            Some(e) if e == study => Box::new(move |p: [f64; 2]| {
                utm.forward(p[0], p[1])
                    .map_or(f64::NAN, |q| d.sample_bilinear(q[0], q[1]))
            }),
            _ => return None,
        };
        Some(f)
    });
    let from = flags
        .from
        .clone()
        .unwrap_or_else(|| SWOT_SCIENCE_START.to_string());
    let to = flags.to.clone().unwrap_or_else(today);
    let level = level_from_sources(
        fetch,
        at.sword_record,
        at.hydrocron,
        &centre,
        utm,
        (&from, &to),
        sampler.as_deref(),
        &params,
        datum,
    )
    .map_err(|e| input_error(e.to_string()))?;
    let mark = if params == LevelParams::default() {
        Mark::Default
    } else {
        Mark::User
    };
    let dem_notice = copernicus.then_some(COPERNICUS_NOTICE);
    record_level(&mut session, &level, mark, dem_notice, &today())
        .map_err(|e| input_error(e.to_string()))?;
    let dest = destination(&flags.doc, &flags.output);
    write(&dest, &session.to_text())?;

    let swot: Vec<_> = level
        .stretches
        .iter()
        .filter_map(|s| match &s.source {
            Source::Swot {
                reach_id,
                observations,
                ..
            } => Some((reach_id.clone(), *observations, s.from_m, s.to_m)),
            Source::Dem => None,
        })
        .collect();
    let dem_stretches = level
        .stretches
        .iter()
        .filter(|s| s.source == Source::Dem)
        .count();
    let mut human = format!(
        "wrote {dest}: water level of {} point(s) along the river centreline ({}, the {}th percentile, a provisional default)",
        level.points.len(),
        datum.name(),
        params.percentile
    );
    for (id, n, a, b) in &swot {
        human.push_str(&format!(
            "\n  {a:.0} to {b:.0} m: SWOT, SWORD reach {id}, {n} observation(s)"
        ));
    }
    for s in level.stretches.iter().filter(|s| s.source == Source::Dem) {
        human.push_str(&format!(
            "\n  {:.0} to {:.0} m: DEM water surface, never rising downstream ({})",
            s.from_m,
            s.to_m,
            s.why.as_deref().unwrap_or("")
        ));
    }
    if !level.lowered.is_empty() {
        human.push_str(&format!(
            "\n  {} DEM level(s) lowered to the level just upstream, so the level never rises downstream (listed in the provenance)",
            level.lowered.len()
        ));
    }
    if let Some((a, b)) = &level.swot_dates {
        human.push_str(&format!("\nSWOT observations from {a} to {b}"));
    }
    human.push_str(&format!(
        "\nThe river flows toward {} chainage ({})",
        if level.downstream_is_increasing {
            "increasing"
        } else {
            "decreasing"
        },
        level.direction_from
    ));
    say(
        dest == "-",
        flags.json,
        &human,
        json!({
            "ok": true, "file": dest, "points": level.points.len(),
            "percentile": params.percentile, "datum": datum.name(), "mark": if mark == Mark::Default { "default" } else { "user" },
            "swot_reaches": swot.iter().map(|(id, n, a, b)| json!({"sword_reach": id, "observations": n, "from_m": a, "to_m": b})).collect::<Vec<_>>(),
            "dem_stretches": dem_stretches,
            "lowered": level.lowered.len(),
            "swot_dates": level.swot_dates.as_ref().map(|(a, b)| format!("{a}/{b}")),
            "downstream_is_increasing": level.downstream_is_increasing,
        }),
    );
    Ok(())
}

//! The quantity schedule (spec 0014, report 2): for each design line option, the dredge and dry cut
//! inside each constraint stretch, the volumes stage's totals, and the cut face. Stretch quantities
//! are the ones the volumes stage stored in its screening (spec 0007 C); the tallest face in a
//! stretch is read from the station walk the volumes stage sums.

use crate::Context;
use crate::fmt::{km, m3, md, num, text, val};
use owd_core::volumes::sections::SectionVolumes;
use owd_format::design::DemCheck;
use owd_format::design::terrain::dem_sampler;
use serde_json::{Value, json};

/// The station walk of an option on the design's DEM, or why there is none.
fn walk(ctx: &Context, id: &str) -> Result<SectionVolumes, String> {
    let name = ctx.session.dem_path().unwrap_or("DEM").to_string();
    match ctx.session.dem_check(ctx.dem) {
        DemCheck::Matches => {}
        DemCheck::Changed => {
            return Err(format!(
                "the DEM `{name}` has changed since the volumes ran"
            ));
        }
        DemCheck::Missing => return Err(format!("the DEM `{name}` was not given")),
        DemCheck::NoDem => return Err("the design has no DEM".into()),
    }
    let grid = owd_format::geotiff::read_bytes(ctx.dem.unwrap_or_default())
        .map_err(|e| format!("{name}: {e}"))?;
    let epsg = ctx.doc["project"]["crs"]["epsg"].as_u64().map(|c| c as u32);
    let sampler = dem_sampler(&grid, epsg).map_err(|e| e.to_string())?;
    ctx.session
        .option_section_walk(id, &sampler)
        .map_err(|e| e.to_string())
}

/// The tallest face of the stations from `from_m` to `to_m`, and its chainage.
fn tallest(walk: &SectionVolumes, from_m: f64, to_m: f64) -> Option<(f64, f64)> {
    walk.stations
        .iter()
        .filter(|s| s.chainage_m >= from_m && s.chainage_m <= to_m && s.max_face_h_m.is_finite())
        .fold(None, |best: Option<(f64, f64)>, s| match best {
            Some((h, _)) if h >= s.max_face_h_m => best,
            _ => Some((s.max_face_h_m, s.chainage_m)),
        })
}

fn table(
    id: String,
    caption: String,
    columns: &[(&str, &str, Option<&str>)],
    rows: Vec<Vec<String>>,
) -> Value {
    let cols: Vec<Value> = columns
        .iter()
        .map(|(t, a, u)| {
            let mut c = json!({"title": t, "align": a});
            if let Some(u) = u {
                c["unit"] = json!(u);
            }
            c
        })
        .collect();
    json!({"type": "table", "id": id, "caption": caption, "columns": cols, "rows": rows})
}

fn prose(m: String) -> Value {
    json!({"type": "prose", "markdown": m})
}

/// The body (the Data and limits box, then one section per design line option), and no appendix.
pub fn schedule(ctx: &Context) -> (Vec<Value>, Vec<Value>) {
    let mut body = vec![
        json!({"type": "section", "id": "data", "title": "Data and limits",
        "numbered": false, "body": [crate::limits::data_box(ctx)]}),
    ];
    body.extend(options(ctx));
    (body, Vec::new())
}

fn options(ctx: &Context) -> Vec<Value> {
    let options = ctx.doc["options"].as_array().cloned().unwrap_or_default();
    if options.is_empty() {
        return vec![
            json!({"type": "section", "id": "no-options", "title": "Quantities", "body": [
                prose("The design has no design line option, so it has no quantities.".into()),
            ]}),
        ];
    }
    let status = ctx
        .session
        .status_with_dem(ctx.dem)
        .stages
        .iter()
        .find(|(s, _)| *s == owd_format::design::Stage::Volumes)
        .map_or("not run", |(_, x)| x.name());
    options
        .iter()
        .enumerate()
        .map(|(i, o)| option_section(ctx, i, o, status))
        .collect()
}

fn option_section(ctx: &Context, i: usize, o: &Value, status: &str) -> Value {
    let id = text(&o["id"], "");
    let name = text(&o["name"], id);
    let v = ctx.doc["stages"]["volumes"]["results"]["options"]
        .as_array()
        .and_then(|a| a.iter().find(|r| r["id"] == o["id"]))
        .cloned()
        .unwrap_or(Value::Null);
    let walk = walk(ctx, id);
    let mut body = vec![prose(format!(
        "The quantities of the volumes stage for **{}**. The volumes stage is **{status}**.{}",
        md(name),
        ctx.session
            .depth_note()
            .map_or(String::new(), |n| format!(" The depth is {}.", md(n)))
    ))];

    // The whole option.
    let (face, face_at) = match (&walk, v["max_face_h_m"].as_f64()) {
        (Ok(w), Some(_)) => match tallest(w, f64::NEG_INFINITY, f64::INFINITY) {
            Some((_, at)) => (val(&v["max_face_h_m"], 2, "not computed"), km(at)),
            None => (
                val(&v["max_face_h_m"], 2, "not computed"),
                "not computed".into(),
            ),
        },
        _ => (
            val(&v["max_face_h_m"], 2, "not computed"),
            "not computed".into(),
        ),
    };
    body.push(table(
        format!("option-{i}-totals"),
        format!("The whole option {name}"),
        &[
            ("Dredge", "right", Some("m³")),
            ("Dry cut", "right", Some("m³")),
            ("Total", "right", Some("m³")),
            ("Tallest cut face", "right", Some("m")),
            ("At chainage", "right", Some("km")),
        ],
        vec![vec![
            m3(&v["dredge_m3"]),
            m3(&v["excav_m3"]),
            m3(&v["total_m3"]),
            face,
            face_at,
        ]],
    ));

    // The constraint stretches.
    let mut list = v["screening"]["constraint_stretches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        body.push(prose("The option has no constraint stretch.".into()));
    } else {
        list.sort_by(|a, b| {
            let c = |s: &Value| s["start_chainage_m"].as_f64().unwrap_or(0.0);
            c(a).total_cmp(&c(b))
        });
        let rows: Vec<Vec<String>> = list
            .iter()
            .map(|s| {
                let from = s["start_chainage_m"].as_f64().unwrap_or(0.0);
                let to = s["end_chainage_m"].as_f64().unwrap_or(from);
                let face = match &walk {
                    Ok(w) => tallest(w, from, to).map_or("—".into(), |(h, _)| num(h, 2)),
                    Err(_) => "not computed".into(),
                };
                vec![
                    text(&s["kind"], "?").to_string(),
                    km(from),
                    km(to),
                    match &s["works"] {
                        Value::String(w) => w.clone(),
                        _ => "none possible".into(),
                    },
                    m3(&s["dredge_m3"]),
                    m3(&s["dry_cut_m3"]),
                    face,
                ]
            })
            .collect();
        let mut t = table(
            format!("option-{i}-stretches"),
            format!("Dredge and dry cut by constraint stretch, option {name}"),
            &[
                ("Kind", "left", None),
                ("From", "right", Some("km")),
                ("To", "right", Some("km")),
                ("Works", "left", None),
                ("Dredge", "right", Some("m³")),
                ("Dry cut", "right", Some("m³")),
                ("Tallest face", "right", Some("m")),
            ],
            rows,
        );
        let mut notes = String::from(
            "Stretches of different kinds overlap, and the stretches do not cover the whole line, so \
             their quantities do not add up to the option's totals above. The tallest face is the \
             tallest at the stations of the volumes stage's walk inside the stretch (\"—\" when no \
             station falls inside it).",
        );
        if let Err(e) = &walk {
            notes.push_str(&format!(" The faces are not computed: {}.", md(e)));
        }
        t["notes"] = json!(notes);
        body.push(t);
    }
    json!({"type": "section", "id": format!("option-{i}"), "title": format!("Option {name}"), "body": body})
}

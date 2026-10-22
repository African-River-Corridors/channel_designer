//! The `owd` subcommands. File, stdin, stdout and clock access live here.

use owd_format::design::alignment::WIDTH_ONLY;
use owd_format::design::export::{failed_checks, water_note};
use owd_format::design::geopackage;
use owd_format::design::{
    AlignMethod, DemCheck, DesignError, Role, Session, Stage, StageStatus, Terrain,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::Path;

/// A failed command: its message and exit code (1 a failed check, 2 a usage or input error).
pub struct Failure {
    pub message: String,
    pub code: u8,
}

pub type Result = core::result::Result<(), Failure>;

pub(crate) fn input_error(message: impl Into<String>) -> Failure {
    Failure {
        message: message.into(),
        code: 2,
    }
}

/// The command that runs a stage.
fn command_for(stage: Stage) -> &'static str {
    match stage {
        Stage::Sizing => "size",
        Stage::Alignment => "align",
        Stage::Optimise => "optimise",
        Stage::Volumes => "volumes",
    }
}

pub(crate) fn from_design(e: DesignError) -> Failure {
    match e {
        DesignError::StaleUpstream { stage, run_first } => Failure {
            message: format!(
                "the {} stage cannot run: {} is stale. Run `owd {}` first (or `owd run`).",
                stage.name(),
                run_first.name(),
                command_for(run_first)
            ),
            code: 1,
        },
        other => Failure {
            message: other.to_string(),
            code: 2,
        },
    }
}

fn read(path: &str) -> core::result::Result<String, Failure> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| input_error(format!("cannot read stdin: {e}")))?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))
    }
}

/// The text of one input: GeoJSON as it is, or a GeoPackage layer turned into GeoJSON. A GeoPackage
/// layer is named `FILE#LAYER`; a file with one feature layer needs no name.
fn read_vector(spec: &str) -> core::result::Result<String, Failure> {
    let (path, layer) = if spec == "-" || Path::new(spec).exists() {
        (spec, None)
    } else {
        match spec.rsplit_once('#') {
            Some((p, l)) if !l.is_empty() => (p, Some(l)),
            _ => (spec, None),
        }
    };
    let bytes = if path == "-" {
        let mut b = Vec::new();
        std::io::stdin()
            .read_to_end(&mut b)
            .map_err(|e| input_error(format!("cannot read stdin: {e}")))?;
        b
    } else {
        std::fs::read(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))?
    };
    if geopackage::is_geopackage(&bytes) {
        geopackage::to_geojson(&bytes, layer).map_err(|e| input_error(format!("{path}: {e}")))
    } else if layer.is_some() {
        Err(input_error(format!(
            "{path}: only a GeoPackage has layers, but this is not one"
        )))
    } else {
        String::from_utf8(bytes)
            .map_err(|_| input_error(format!("{path}: not GeoJSON text or a GeoPackage")))
    }
}

pub(crate) fn write(path: &str, text: &str) -> Result {
    if path == "-" {
        std::io::stdout()
            .write_all(text.as_bytes())
            .map_err(|e| input_error(format!("cannot write stdout: {e}")))
    } else {
        std::fs::write(path, text).map_err(|e| input_error(format!("cannot write {path}: {e}")))
    }
}

pub(crate) fn open(path: &str) -> core::result::Result<Session, Failure> {
    owd_format::design::check_file_name(path).map_err(from_design)?;
    Session::open(&read(path)?).map_err(from_design)
}

/// Where a document's relative paths start: its own folder (the working folder for stdin).
fn doc_dir(doc: &str) -> std::path::PathBuf {
    match Path::new(doc).parent() {
        Some(p) if doc != "-" && !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    }
}

/// The DEM path as the document records it: relative to the document's folder, with `/`.
fn relative_dem_path(doc: &str, dem: &str) -> String {
    let base = std::fs::canonicalize(doc_dir(doc)).ok();
    let full = std::fs::canonicalize(dem).ok();
    match (base, full) {
        (Some(b), Some(f)) => match f.strip_prefix(&b) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => pathdiff(&b, &f),
        },
        _ => dem.to_string(),
    }
}

/// `target` relative to `base` (both absolute), using `..` where it must climb.
fn pathdiff(base: &Path, target: &Path) -> String {
    let b: Vec<_> = base.components().collect();
    let t: Vec<_> = target.components().collect();
    let common = b.iter().zip(&t).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".to_string(); b.len() - common];
    parts.extend(
        t[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// Open a document, find its DEM file and warn on stderr when the DEM is missing or has changed.
/// Returns the session and the DEM file's bytes, if the file exists.
pub(crate) fn open_with_dem(
    doc: &str,
) -> core::result::Result<(Session, Option<Vec<u8>>), Failure> {
    let session = open(doc)?;
    let bytes = session
        .dem_path()
        .and_then(|p| std::fs::read(doc_dir(doc).join(p)).ok());
    if let Some(w) = session.dem_warning(bytes.as_deref()) {
        eprintln!("owd: warning: {w}");
    }
    Ok((session, bytes))
}

/// The DEM as a terrain sampler, for the stages that need one.
pub(crate) fn read_dem(
    bytes: &[u8],
    what: &str,
) -> core::result::Result<owd_format::geotiff::Dem, Failure> {
    owd_format::geotiff::read_bytes(bytes).map_err(|e| input_error(format!("{what}: {e}")))
}

fn project_epsg(session: &Session) -> Option<u32> {
    session.document()["project"]["crs"]["epsg"]
        .as_u64()
        .map(|c| c as u32)
}

/// Print a result. When the document itself goes to stdout the result goes to stderr, so the two
/// never mix.
pub(crate) fn say(doc_on_stdout: bool, json_mode: bool, human: &str, value: Value) {
    let text = if json_mode {
        format!("{value}\n")
    } else {
        format!("{human}\n")
    };
    if doc_on_stdout {
        eprint!("{text}");
    } else {
        print!("{text}");
    }
}

/// Where a command that rewrites a document puts it: `-o`, else back where it came from
/// (stdout if it came from stdin).
pub(crate) fn destination(doc: &str, output: &Option<String>) -> String {
    output.clone().unwrap_or_else(|| doc.to_string())
}

/// Today's date in UTC, `YYYY-MM-DD`, from the system clock.
pub(crate) fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    owd_format::design::date_from_unix(secs)
}

pub fn new(file: &str, title: &str, date: Option<&str>, json_mode: bool) -> Result {
    let date = date.map_or_else(today, str::to_string);
    let ok = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !ok {
        return Err(input_error(format!(
            "`{date}` is not a date: use YYYY-MM-DD"
        )));
    }
    if title.trim().is_empty() {
        return Err(input_error("the title must not be empty"));
    }
    owd_format::design::check_file_name(file).map_err(from_design)?;
    let s = Session::new(title, &date);
    write(file, &s.to_text())?;
    say(
        file == "-",
        json_mode,
        &format!("created {file}: \"{title}\""),
        json!({"ok": true, "file": file, "title": title, "created": date}),
    );
    Ok(())
}

pub fn import(
    doc: &str,
    inputs: [(&str, Option<String>); 4],
    dem: Option<String>,
    water: WaterFlags,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let roles = [
        Role::LeftBank,
        Role::RightBank,
        Role::Centreline,
        Role::RiverPolygon,
    ];
    let mut texts: Vec<(Role, String)> = Vec::new();
    let mut stdin_used = doc == "-";
    for (role, (_, path)) in roles.into_iter().zip(inputs) {
        if let Some(path) = path {
            if path == "-" {
                if stdin_used {
                    return Err(input_error("only one input can be read from stdin"));
                }
                stdin_used = true;
            }
            texts.push((role, read_vector(&path)?));
        }
    }
    let has_water = water.level.is_some() || water.slope.is_some() || water.table.is_some();
    if texts.is_empty() && dem.is_none() && !has_water {
        return Err(input_error(
            "nothing to import: give --left-bank, --right-bank, --centreline, --polygon, --dem or a water level",
        ));
    }
    // The document is read first when it is on stdin: stdin can be read once.
    let mut session = open(doc)?;
    let items: Vec<(Role, &str)> = texts.iter().map(|(r, t)| (*r, t.as_str())).collect();
    let mut report = if items.is_empty() {
        owd_format::design::ImportReport {
            project_epsg: project_epsg(&session).unwrap_or(0),
            inputs: Vec::new(),
        }
    } else {
        session.import(&items).map_err(from_design)?
    };
    apply_water(&mut session, &water)?;
    if let Some(path) = &dem {
        let bytes =
            std::fs::read(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))?;
        let d = read_dem(&bytes, path)?;
        // Refuse a DEM the stages could not sample, now, with the CRS named.
        owd_format::design::terrain::dem_sampler(&d, project_epsg(&session))
            .map(|_| ())
            .or_else(|e| {
                // No project CRS yet is fine: the river comes later.
                if project_epsg(&session).is_none() {
                    Ok(())
                } else {
                    Err(e)
                }
            })
            .map_err(from_design)?;
        session.set_dem(&relative_dem_path(doc, path), &bytes, &d);
        report.inputs.push((
            "dem".to_string(),
            owd_format::design::terrain::dem_crs_name(&d),
            d.width * d.height,
        ));
    }
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let human = {
        let mut lines = vec![format!("project CRS: EPSG:{}", report.project_epsg)];
        for (name, src, n) in &report.inputs {
            let unit = if name == "dem" { "pixels" } else { "positions" };
            lines.push(format!("{name}: {n} {unit}, from {src}"));
        }
        lines.join("\n")
    };
    let inputs_json: Vec<Value> = report
        .inputs
        .iter()
        .map(|(n, c, k)| json!({"input": n, "source_crs": c, "positions": k}))
        .collect();
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "project_epsg": report.project_epsg, "inputs": inputs_json}),
    );
    Ok(())
}

/// The choices of `owd size`.
pub struct SizeFlags {
    pub rule_sets: Vec<String>,
    pub set_inputs: Vec<String>,
    pub files: Vec<String>,
    pub ruleset: Option<String>,
    pub vessel: Option<String>,
    pub draught: Option<f64>,
}

/// Load each Rule Set file into the session, through the loader the built-in sets take.
fn load_rule_set_files(session: &mut Session, files: &[String]) -> Result {
    for f in files {
        let text = std::fs::read_to_string(f)
            .map_err(|e| input_error(format!("cannot read the Rule Set file `{f}`: {e}")))?;
        session
            .load_rule_set(&text)
            .map_err(|e| input_error(format!("{f}: {e}")))?;
    }
    Ok(())
}

pub fn size(doc: &str, flags: SizeFlags, output: &Option<String>, json_mode: bool) -> Result {
    // Opened first: when the document comes from stdin it must be read before anything else.
    let mut session = open(doc)?;
    load_rule_set_files(&mut session, &flags.files)?;
    let settings = &session.document()["stages"]["sizing"]["settings"];
    let vessel = match (&flags.vessel, settings["vessel_standard_id"].as_str()) {
        (Some(v), _) => v.clone(),
        (None, Some(v)) => v.to_string(),
        (None, None) => return Err(input_error("choose a vessel class with --vessel")),
    };
    let draught = flags.draught.or_else(|| {
        (flags.vessel.is_none())
            .then(|| settings["draught_m"].as_f64())
            .flatten()
    });
    match &flags.ruleset {
        Some(old) => session
            .set_sizing(old, &vessel, draught)
            .map_err(from_design)?,
        None => {
            let mut inputs = Vec::new();
            for kv in &flags.set_inputs {
                let (k, v) = kv.split_once('=').ok_or_else(|| {
                    input_error(format!("--set-input `{kv}`: write it as NAME=VALUE"))
                })?;
                inputs.push((k.trim(), v.trim()));
            }
            let ids: Vec<&str> = flags.rule_sets.iter().map(String::as_str).collect();
            session.set_rule_sets(&ids, &inputs).map_err(from_design)?;
            session.set_vessel(&vessel, draught).map_err(from_design)?;
        }
    }
    session.run_stage(Stage::Sizing).map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let shape = session.document()["stages"]["sizing"]["results"].clone();
    let f = |k: &str| shape[k].as_f64().unwrap_or(f64::NAN);
    let mut human = format!(
        "{} x {}: bottom width {:.1} m, R_min {:.0} m, design depth {:.2} m",
        shape["ruleset_id"].as_str().unwrap_or(""),
        vessel,
        f("bottom_width_straight_m"),
        f("min_bend_radius_m"),
        f("design_depth_m")
    );
    for w in shape["warnings"].as_array().into_iter().flatten() {
        if let Some(w) = w.as_str() {
            human.push_str(&format!("\nwarning: {w}"));
        }
    }
    human
        .push_str("\nEvery Rule is unverified until a second reader checks it against its source.");
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "sizing": shape}),
    );
    Ok(())
}

/// `owd trace`: the full Rule Trace, rebuilt from the document.
pub fn trace(
    doc: &str,
    option: Option<&str>,
    station_m: f64,
    files: &[String],
    json_mode: bool,
) -> Result {
    if !(station_m.is_finite() && station_m > 0.0) {
        return Err(input_error(
            "--station-m must be a positive number of metres",
        ));
    }
    let mut session = open(doc)?;
    load_rule_set_files(&mut session, files)?;
    let t = session.rule_trace(option, station_m).map_err(from_design)?;
    let mut human = format!(
        "# {}\n# {}\n# sets: {}\n",
        t["notice"].as_str().unwrap_or(""),
        t["rules_notice"].as_str().unwrap_or(""),
        t["sets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    );
    human.push_str("rule\tobject\tvalue\tcompare\tlimit\tmargin\toutcome\tgoverning\tstatus\n");
    let cell = |v: &Value| match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    for r in t["rows"].as_array().into_iter().flatten() {
        human.push_str(
            &[
                cell(&r["rule"]),
                cell(&r["object"]),
                cell(&r["value"]),
                cell(&r["compare"]),
                cell(&r["limit"]),
                cell(&r["margin"]),
                cell(&r["outcome"]),
                if r["governing"] == true {
                    "governing".into()
                } else {
                    String::new()
                },
                cell(&r["status"]),
            ]
            .join("\t"),
        );
        human.push('\n');
    }
    say(false, json_mode, human.trim_end(), t);
    Ok(())
}

/// `owd waterway add`: mark a reach of the river.
pub fn waterway_add(
    doc: &str,
    from: f64,
    to: Option<f64>,
    waterway: &str,
    claim: bool,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let mut session = open(doc)?;
    session
        .add_waterway_reach(from, to, waterway, claim)
        .map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let human = format!(
        "marked {} m to {} as {waterway}{}; the sizing stage is now stale",
        from,
        to.map_or("the end of the river".to_string(), |t| format!("{t} m")),
        if claim {
            ", with the reduced-radius claim"
        } else {
            ""
        }
    );
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "waterway_reaches": session.waterway_reaches()}),
    );
    Ok(())
}

/// `owd waterway list`.
pub fn waterway_list(doc: &str, json_mode: bool) -> Result {
    let session = open(doc)?;
    let reaches = session.waterway_reaches();
    let human = if reaches.is_empty() {
        "no reach marked: the whole river is a free-flowing river with no claim".to_string()
    } else {
        reaches
            .iter()
            .map(|r| {
                format!(
                    "{} m to {}: {}, reduced_radius_claim {}",
                    r["start_m"],
                    r["end_m"]
                        .as_f64()
                        .map_or("the end".to_string(), |e| format!("{e} m")),
                    r["waterway"].as_str().unwrap_or(""),
                    r["reduced_radius_claim"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    say(
        false,
        json_mode,
        &human,
        json!({"waterway_reaches": reaches}),
    );
    Ok(())
}

/// `owd waterway remove`.
pub fn waterway_remove(doc: &str, from: f64, output: &Option<String>, json_mode: bool) -> Result {
    let mut session = open(doc)?;
    session.remove_waterway_reach(from).map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    say(
        dest == "-",
        json_mode,
        &format!("removed the reach from {from} m"),
        json!({"ok": true, "waterway_reaches": session.waterway_reaches()}),
    );
    Ok(())
}

/// `owd rules`: the Rule Sets, one set with its Rules (`show ID`), or the Measures (`--measures`).
pub fn rules(
    action: Option<&str>,
    id: Option<&str>,
    measures: bool,
    files: &[String],
    json_mode: bool,
) -> Result {
    use owd_core::rules::engine;
    let mut session = Session::new("", "");
    load_rule_set_files(&mut session, files)?;
    let lib = session.rule_set_library();
    if measures {
        let human: Vec<String> = engine::MEASURES
            .iter()
            .map(|(m, what)| format!("{m} (m): {what}"))
            .collect();
        let js: Vec<Value> = engine::MEASURES
            .iter()
            .map(|(m, what)| json!({"id": m, "unit": "m", "object": engine::CHANNEL, "what": what}))
            .collect();
        say(false, json_mode, &human.join("\n"), json!({"measures": js}));
        return Ok(());
    }
    match (action, id) {
        (None, None) => {
            let human: Vec<String> = lib
                .sets()
                .iter()
                .map(|s| {
                    format!(
                        "{}  {}  ({} Rules, hash {})",
                        s.key(),
                        s.title,
                        s.rules.len(),
                        &s.hash[..12]
                    )
                })
                .collect();
            let js: Vec<Value> = lib
                .sets()
                .iter()
                .map(|s| json!({"id": s.id, "version": s.version, "title": s.title, "hash": s.hash, "rules": s.rules.len()}))
                .collect();
            say(
                false,
                json_mode,
                &human.join("\n"),
                json!({"rule_sets": js}),
            );
            Ok(())
        }
        (Some("show"), Some(id)) => {
            let set = lib
                .lookup(id)
                .ok_or_else(|| input_error(format!("unknown Rule Set `{id}`; see `owd rules`")))?;
            let rules: Vec<Value> = set
                .rules
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "title": r.title,
                        "origin": r.origin.key(),
                        "status": if r.verified.is_some() { "verified" } else { "unverified" },
                        "measure": r.measure.id,
                        "consequence": r.consequence.key(),
                        "sizes": r.sizes,
                        "evaluated": r.evaluated,
                        "finding": r.finding,
                    })
                })
                .collect();
            say(
                false,
                json_mode,
                arc_rules::load::describe(set).trim_end(),
                json!({"id": set.id, "version": set.version, "title": set.title, "hash": set.hash, "rules": rules}),
            );
            Ok(())
        }
        _ => Err(input_error(
            "use `owd rules`, `owd rules show ID` or `owd rules --measures`",
        )),
    }
}

/// The water level flags: a constant, a straight fall along the river chainage, or a table.
#[derive(Default)]
pub struct WaterFlags {
    pub level: Option<f64>,
    pub slope: Option<f64>,
    pub table: Option<String>,
}

/// Set the document's water level profile from the flags, if any were given. A level alone is a
/// constant; with a slope it is the level at the river's start, falling that much per metre.
fn apply_water(session: &mut Session, w: &WaterFlags) -> Result {
    if let Some(path) = &w.table {
        if w.level.is_some() || w.slope.is_some() {
            return Err(input_error(
                "give the water level as a table or as a level and a slope, not both",
            ));
        }
        let text = read(path)?;
        let mut points = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            match (
                f.first().and_then(|x| x.parse::<f64>().ok()),
                f.get(1).and_then(|x| x.parse::<f64>().ok()),
            ) {
                (Some(c), Some(l)) if f.len() == 2 => points.push([c, l]),
                _ if n == 0 => {} // a header line
                _ => {
                    return Err(input_error(format!(
                        "{path}, line {}: expected `chainage_m,level_m`",
                        n + 1
                    )));
                }
            }
        }
        return session.set_water_profile(&points).map_err(from_design);
    }
    match (w.level, w.slope) {
        (None, None) => Ok(()),
        (Some(l), None) => session.set_water_profile(&[[0.0, l]]).map_err(from_design),
        (Some(l), Some(k)) => session.set_water_fall(l, k).map_err(from_design),
        (None, Some(_)) => Err(input_error(
            "--water-slope needs --water-level (the level at the start)",
        )),
    }
}

/// The flags of `owd align`.
pub struct AlignFlags {
    pub step: Option<f64>,
    pub passes: Option<u32>,
    pub chord: Option<f64>,
    pub dem: Option<String>,
    pub water_level: Option<f64>,
    pub water_slope: Option<f64>,
    pub corridors: Option<String>,
    pub fairness_weight: Option<f64>,
}

pub fn align(
    doc: &str,
    method: &str,
    flags: AlignFlags,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let method = AlignMethod::from_name(method).ok_or_else(|| {
        input_error(format!(
            "unknown method `{method}`: use rolling-circle or reroute"
        ))
    })?;
    let mut session = open(doc)?;
    let mut overrides = serde_json::Map::new();
    let mut put = |k: &str, v: Value| {
        overrides.insert(k.to_string(), v);
    };
    match method {
        AlignMethod::RollingCircle => {
            if let Some(v) = flags.step {
                put("step_m", json!(v));
            }
            if let Some(v) = flags.passes {
                put("passes", json!(v));
            }
        }
        AlignMethod::Reroute => {
            if let Some(list) = &flags.corridors {
                let parsed: core::result::Result<Vec<f64>, _> =
                    list.split(',').map(|x| x.trim().parse::<f64>()).collect();
                let parsed = parsed
                    .map_err(|_| input_error(format!("--corridors must be numbers: `{list}`")))?;
                put("corridor_sweep_m", json!(parsed));
            }
            if let Some(v) = flags.fairness_weight {
                put("fairness_weight", json!(v));
            }
        }
    }
    if let Some(v) = flags.chord {
        put("check_chord_m", json!(v));
    }
    let overrides = Value::Object(overrides);
    if method == AlignMethod::Reroute {
        apply_water(
            &mut session,
            &WaterFlags {
                level: flags.water_level,
                slope: flags.water_slope,
                table: None,
            },
        )?;
    } else if flags.water_level.is_some() || flags.water_slope.is_some() {
        return Err(input_error(
            "--water-level and --water-slope are for the re-route only",
        ));
    }
    // The re-route samples a DEM; read it here, since the session has no file access. A DEM given
    // here becomes the document's DEM reference.
    let dem = match (&flags.dem, method) {
        (Some(path), AlignMethod::Reroute) => {
            let bytes =
                std::fs::read(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))?;
            let dem = read_dem(&bytes, path)?;
            session.set_dem(&relative_dem_path(doc, path), &bytes, &dem);
            Some(dem)
        }
        (Some(_), _) => return Err(input_error("--dem is for the re-route only")),
        _ => None,
    };
    let sampler = match &dem {
        Some(d) => Some(
            owd_format::design::terrain::dem_sampler(d, project_epsg(&session))
                .map_err(from_design)?,
        ),
        None => None,
    };
    session
        .align(
            method,
            &overrides,
            sampler
                .as_ref()
                .map(|s| s as &dyn Fn(&[f64], &[f64]) -> Vec<f64>),
        )
        .map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let option = session.document()["options"]
        .as_array()
        .and_then(|o| o.iter().find(|o| o["method"] == method.name()))
        .cloned()
        .unwrap_or(Value::Null);
    let c = &option["checks"];
    let failed = failed_checks(&option);
    let ok = failed.is_empty();
    let mut human = format!(
        "{}: minimum radius {:.0} m on a {:.0} m chord against R_min {:.0} m: {}",
        option["name"].as_str().unwrap_or(method.name()),
        c["measured_min_radius_m"].as_f64().unwrap_or(f64::NAN),
        c["radius_check_chord_m"].as_f64().unwrap_or(f64::NAN),
        c["r_min_m"].as_f64().unwrap_or(f64::NAN),
        if c["radius_ok"].as_bool() == Some(true) {
            "ok"
        } else {
            "FAILS R_min"
        }
    );
    if let Some(st) = c["fits_in_water"]["stretches"].as_array() {
        human.push_str(&format!(
            "\nfits in water ({}): {}",
            WIDTH_ONLY,
            if st.is_empty() {
                "ok".to_string()
            } else {
                format!("FAILS at {} stretch(es)", st.len())
            }
        ));
    }
    if c["join_guard"]["ok"].as_bool() == Some(false) {
        human.push_str("\njoin guard: FAILS");
    }
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "option": option}),
    );
    // An option that fails a check is a failed check: exit code 1, with the document written.
    if ok {
        Ok(())
    } else {
        Err(Failure {
            message: format!("the option fails: {}", failed.join(", ")),
            code: 1,
        })
    }
}

/// The DEM for a stage that needs one. A DEM given with `--dem` is read and recorded as the
/// document's reference. Otherwise the one the document names must be there and must be the file
/// whose hash was recorded.
fn stage_dem(
    session: &mut Session,
    doc: &str,
    bytes: Option<&[u8]>,
    flag: &Option<String>,
    command: &str,
) -> core::result::Result<Option<owd_format::geotiff::Dem>, Failure> {
    if let Some(path) = flag {
        let b = std::fs::read(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))?;
        let d = read_dem(&b, path)?;
        session.set_dem(&relative_dem_path(doc, path), &b, &d);
        return Ok(Some(d));
    }
    let p = session.dem_path().unwrap_or("DEM").to_string();
    match session.dem_check(bytes) {
        DemCheck::Matches => Ok(Some(read_dem(bytes.unwrap_or_default(), &p)?)),
        DemCheck::Changed => Err(input_error(format!(
            "the DEM `{p}` has changed since its hash was recorded, so the {command} stage will not run on it. \
             To accept the new file run `owd {command} --dem {p}`."
        ))),
        DemCheck::Missing => Err(input_error(format!(
            "the DEM `{p}` is missing: the {command} stage cannot run without it. \
             Put the file back, or point to another with `owd {command} --dem FILE`."
        ))),
        DemCheck::NoDem => Ok(None),
    }
}

/// The flags of `owd optimise`.
pub struct OptimiseFlags {
    pub dem: Option<String>,
    pub base_level: Option<f64>,
    pub sites: Option<String>,
    pub lifts: Option<String>,
    pub wet_weight: Option<f64>,
    pub dry_weight: Option<f64>,
    pub model: Option<String>,
}

fn number_list(flag: &str, list: &str) -> core::result::Result<Vec<f64>, Failure> {
    list.split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<core::result::Result<Vec<f64>, _>>()
        .map_err(|_| input_error(format!("{flag} must be numbers, comma separated: `{list}`")))
}

pub fn optimise(
    doc: &str,
    flags: OptimiseFlags,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    let (mut session, bytes) = open_with_dem(doc)?;
    let mut overrides = serde_json::Map::new();
    for (k, v) in [
        ("base_level_m", flags.base_level),
        ("wet_weight", flags.wet_weight),
        ("dry_weight", flags.dry_weight),
    ] {
        if let Some(v) = v {
            overrides.insert(k.into(), json!(v));
        }
    }
    if let Some(m) = &flags.model {
        overrides.insert("cut_model".into(), json!(m));
    }
    if let Some(l) = &flags.sites {
        overrides.insert(
            "candidate_sites_m".into(),
            json!(number_list("--sites", l)?),
        );
    }
    if let Some(l) = &flags.lifts {
        overrides.insert("lifts_m".into(), json!(number_list("--lifts", l)?));
    }
    let dem = stage_dem(&mut session, doc, bytes.as_deref(), &flags.dem, "optimise")?;
    let sampler = match &dem {
        Some(d) => Some(
            owd_format::design::terrain::dem_sampler(d, project_epsg(&session))
                .map_err(from_design)?,
        ),
        None => None,
    };
    session
        .run_optimise(
            &Value::Object(overrides),
            sampler.as_ref().map(|s| s as Terrain),
        )
        .map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let res = session.document()["stages"]["optimise"]["results"].clone();
    let option = session.document()["options"]
        .as_array()
        .and_then(|o| o.iter().find(|o| o["method"] == "lattice"))
        .cloned()
        .unwrap_or(Value::Null);
    let weirs: Vec<String> = res["cascade"]["weirs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| {
            format!(
                "{:.0} m -> pool {:.1} m",
                w["chainage_m"].as_f64().unwrap_or(f64::NAN),
                w["pool_level_m"].as_f64().unwrap_or(f64::NAN)
            )
        })
        .collect();
    let c = &option["checks"];
    let ok = c["radius_ok"].as_bool().unwrap_or(false);
    let human = format!(
        "weirs: {}\nPool lattice: {} pool(s) re-routed, minimum radius {:.0} m on a {:.0} m chord against R_min {:.0} m: {}",
        if weirs.is_empty() {
            "none".to_string()
        } else {
            weirs.join(", ")
        },
        option["details"]["n_replaced"],
        c["measured_min_radius_m"].as_f64().unwrap_or(f64::NAN),
        c["radius_check_chord_m"].as_f64().unwrap_or(f64::NAN),
        c["r_min_m"].as_f64().unwrap_or(f64::NAN),
        if ok { "ok" } else { "FAILS R_min" }
    );
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "cascade": res["cascade"], "pools": res["pools"], "option": option}),
    );
    if ok {
        Ok(())
    } else {
        Err(Failure {
            message: "the option fails R_min".into(),
            code: 1,
        })
    }
}

/// The flags of `owd volumes`.
pub struct VolumesFlags {
    pub dem: Option<String>,
    pub model: Option<String>,
    pub water_level: Option<f64>,
    pub water_slope: Option<f64>,
    pub station: Option<f64>,
    pub maxw: Option<f64>,
}

pub fn volumes(doc: &str, flags: VolumesFlags, output: &Option<String>, json_mode: bool) -> Result {
    let (mut session, bytes) = open_with_dem(doc)?;
    let mut overrides = serde_json::Map::new();
    if let Some(m) = &flags.model {
        overrides.insert("cut_model".into(), json!(m));
    }
    for (k, v) in [("station_m", flags.station), ("maxw_m", flags.maxw)] {
        if let Some(v) = v {
            overrides.insert(k.into(), json!(v));
        }
    }
    apply_water(
        &mut session,
        &WaterFlags {
            level: flags.water_level,
            slope: flags.water_slope,
            table: None,
        },
    )?;
    let dem = stage_dem(&mut session, doc, bytes.as_deref(), &flags.dem, "volumes")?;
    let sampler = match &dem {
        Some(d) => Some(
            owd_format::design::terrain::dem_sampler(d, project_epsg(&session))
                .map_err(from_design)?,
        ),
        None => None,
    };
    session
        .run_volumes(
            &Value::Object(overrides),
            sampler.as_ref().map(|s| s as Terrain),
        )
        .map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let rows = session.document()["stages"]["volumes"]["results"]["options"].clone();
    let mut human = Vec::new();
    let mut all_ok = true;
    for r in rows.as_array().into_iter().flatten() {
        let ok = r["radius_ok"].as_bool().unwrap_or(false);
        all_ok &= ok;
        let f = |k: &str| r[k].as_f64().unwrap_or(f64::NAN);
        human.push(format!(
            "{}: dredge {:.0} m3, dry cut {:.0} m3, tallest face {:.1} m{}",
            r["name"].as_str().unwrap_or("?"),
            f("dredge_m3"),
            f("excav_m3"),
            f("max_face_h_m"),
            if ok { "" } else { "  [FAILS R_min]" }
        ));
    }
    say(
        dest == "-",
        json_mode,
        &human.join("\n"),
        json!({"ok": true, "options": rows}),
    );
    if all_ok {
        Ok(())
    } else {
        Err(Failure {
            message: "an option fails R_min".into(),
            code: 1,
        })
    }
}

/// Largest number of tiles one `fetch-dem` will take: a reach this wide wants a DEM of its own.
const MAX_TILES: usize = 16;

pub fn fetch_dem(
    doc: &str,
    margin: f64,
    mirror: Option<&str>,
    bbox: Option<&str>,
    json_mode: bool,
) -> Result {
    fetch_dem_with(doc, margin, mirror, bbox, json_mode, &crate::fetch::Mirror)
}

/// `--bbox west,south,east,north` in degrees.
fn parse_bbox(s: &str) -> core::result::Result<[f64; 4], Failure> {
    let bad = || {
        input_error(format!(
            "`{s}` is not `west,south,east,north` in degrees (for example -6.2,4.9,-5.9,5.2)"
        ))
    };
    let v: Vec<f64> = s
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<core::result::Result<_, _>>()
        .map_err(|_| bad())?;
    let [w, so, e, n] = v[..] else {
        return Err(bad());
    };
    if !(w < e
        && so < n
        && w.abs() <= 180.0
        && e.abs() <= 180.0
        && so.abs() <= 90.0
        && n.abs() <= 90.0)
    {
        return Err(bad());
    }
    Ok([w, so, e, n])
}

/// `fetch-dem` with the fetcher given, so a test can serve a recorded response.
pub fn fetch_dem_with(
    doc: &str,
    margin: f64,
    mirror: Option<&str>,
    bbox: Option<&str>,
    json_mode: bool,
    fetcher: &dyn crate::fetch::Fetcher,
) -> Result {
    use owd_format::design::dem_source::{
        COPERNICUS_NOTICE, DEFAULT_MIRROR, SOURCE_NAME, mosaic, tiles_for, write_geotiff,
    };
    if doc == "-" {
        return Err(input_error(
            "fetch-dem writes files next to the document: give a file, not `-`",
        ));
    }
    if !(margin.is_finite() && margin >= 0.0) {
        return Err(input_error(
            "--margin must be a number of metres, 0 or more",
        ));
    }
    let mut session = open(doc)?;
    let mirror = mirror.unwrap_or(DEFAULT_MIRROR);
    // The box the DEM must cover: the one given, else the document's river, else its reach.
    let bbox = match bbox {
        Some(b) => {
            let b = owd_reach::dem::widen(parse_bbox(b)?, margin);
            (b[0], b[1], b[2], b[3])
        }
        None => session.extent_wgs84(margin).map_err(from_design)?,
    };
    let tiles = tiles_for(bbox);
    if tiles.len() > MAX_TILES {
        return Err(input_error(format!(
            "the reach needs {} tiles; fetch-dem takes at most {MAX_TILES}. Give a DEM of your own",
            tiles.len()
        )));
    }
    let dir = doc_dir(doc);
    let stem = Path::new(doc)
        .file_stem()
        .map_or("design".to_string(), |s| s.to_string_lossy().into_owned());
    let tiles_dir_name = format!("{stem}-dem");
    let tiles_dir = dir.join(&tiles_dir_name);
    let recorded = session.dem_source_tiles();

    // Look at every file first, so a refusal changes nothing.
    let mut plan = Vec::new();
    for t in &tiles {
        let rel = format!("{tiles_dir_name}/{}.tif", t.name());
        let path = dir.join(&rel);
        let found = match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(input_error(format!("cannot read {rel}: {e}"))),
        };
        if let Some(b) = &found {
            let have = owd_format::design::hash::sha256_hex(b);
            match recorded.iter().find(|r| r.name == t.name()) {
                Some(r) if r.sha256 == have => {}
                Some(r) => {
                    return Err(input_error(format!(
                        "{rel} exists and its hash ({have}) differs from the one recorded ({}). \
                         Refusing to overwrite it: move it away, or restore the file.",
                        r.sha256
                    )));
                }
                None => {
                    return Err(input_error(format!(
                        "{rel} exists but the channel design has no record of it. \
                         Refusing to overwrite it: move it away first."
                    )));
                }
            }
        }
        plan.push((*t, rel, path, found));
    }

    let mut records = Vec::new();
    let mut dems = Vec::new();
    let mut fetched = 0;
    let mut withheld: Vec<owd_format::design::dem_source::TileId> = Vec::new();
    for (t, rel, path, found) in plan {
        let bytes = match found {
            Some(b) => b,
            None => {
                let b = match fetcher.get(t, mirror) {
                    Ok(b) => b,
                    Err(crate::fetch::GetError::Withheld) => {
                        withheld.push(t);
                        continue;
                    }
                    Err(crate::fetch::GetError::Failed(m)) => return Err(input_error(m)),
                };
                std::fs::create_dir_all(&tiles_dir).map_err(|e| {
                    input_error(format!("cannot create {}: {e}", tiles_dir.display()))
                })?;
                std::fs::write(&path, &b)
                    .map_err(|e| input_error(format!("cannot write {rel}: {e}")))?;
                fetched += 1;
                b
            }
        };
        dems.push((read_dem(&bytes, &rel)?, bytes.clone()));
        records.push(crate::fetch::record(t, mirror, &rel, &bytes));
    }

    if !withheld.is_empty() {
        // Named, never skipped: a DEM with a missing tile has a hole.
        return Err(input_error(owd_reach::dem::withheld_message(
            &withheld
                .iter()
                .map(|t| owd_reach::dem::DemTile(*t))
                .collect::<Vec<_>>(),
        )));
    }
    // One tile is the DEM as it is. Tiles on one grid are joined into one file, with no resampling.
    let (dem_rel, dem_bytes, dem) = if dems.len() == 1 {
        let (d, b) = dems.remove(0);
        (records[0].file.clone(), b, d)
    } else {
        let joined = mosaic(&dems.iter().map(|(d, _)| d.clone()).collect::<Vec<_>>())
            .map_err(input_error)?;
        let bytes = write_geotiff(&joined).map_err(input_error)?;
        let rel = format!("{stem}-dem.tif");
        let path = dir.join(&rel);
        if let Ok(existing) = std::fs::read(&path) {
            let have = owd_format::design::hash::sha256_hex(&existing);
            let want = session.document()["inputs"]["dem"]["sha256"].as_str();
            if have != owd_format::design::hash::sha256_hex(&bytes) && want != Some(have.as_str()) {
                return Err(input_error(format!(
                    "{rel} exists and is not the file this channel design recorded. \
                     Refusing to overwrite it: move it away first."
                )));
            }
        }
        std::fs::write(&path, &bytes)
            .map_err(|e| input_error(format!("cannot write {rel}: {e}")))?;
        (rel, bytes, joined)
    };
    session.set_dem(&dem_rel, &dem_bytes, &dem);
    session.set_dem_source(&records).map_err(from_design)?;
    // Every generated input says where it came from (spec 0012): a `default`, with the DEM notice.
    let made = today();
    let rec = owd_reach::provenance::provenance_record(
        "cop-dem-glo30",
        &made,
        owd_format::design::provenance::Mark::Default,
        false,
    )
    .map_err(input_error)?;
    session.set_provenance("dem", rec).map_err(from_design)?;

    let notice = format!(
        "{SOURCE_NAME}\n\n{COPERNICUS_NOTICE}\n\nTiles:\n{}\n",
        records
            .iter()
            .map(|r| format!(
                "  {}  sha256 {}  {} bytes  {}",
                r.file, r.sha256, r.bytes, r.url
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
    std::fs::create_dir_all(&tiles_dir)
        .map_err(|e| input_error(format!("cannot create {}: {e}", tiles_dir.display())))?;
    write(&tiles_dir.join("NOTICE.txt").to_string_lossy(), &notice)?;
    write(doc, &session.to_text())?;
    let human = format!(
        "{} tile(s) for the reach, {} downloaded; DEM: {dem_rel}\n{COPERNICUS_NOTICE}",
        records.len(),
        fetched
    );
    say(
        false,
        json_mode,
        &human,
        json!({
            "ok": true, "dem": dem_rel, "downloaded": fetched,
            "tiles": records.iter().map(|r| json!({
                "name": r.name, "file": r.file, "sha256": r.sha256, "bytes": r.bytes, "url": r.url,
            })).collect::<Vec<_>>(),
            "licence": COPERNICUS_NOTICE,
        }),
    );
    Ok(())
}

pub fn export(
    doc: &str,
    geojson: bool,
    csv: bool,
    wgs84: bool,
    stretches: bool,
    output: Option<&str>,
) -> Result {
    if !geojson && !csv {
        return Err(input_error("choose an export: --geojson or --csv"));
    }
    let (session, bytes) = open_with_dem(doc)?;
    // Volumes numbers that no longer hold are exported with their status, never as fresh.
    let text = match (geojson, stretches) {
        (true, false) => session
            .export_geojson(wgs84, bytes.as_deref())
            .map_err(from_design)?,
        (false, false) => session.export_csv(bytes.as_deref()),
        (true, true) => session
            .export_stretches_geojson(wgs84, bytes.as_deref())
            .map_err(from_design)?,
        (false, true) => session.export_stretches_csv(bytes.as_deref()),
    };
    write(output.unwrap_or("-"), &text)
}

/// The flags of `owd draw`.
pub struct DrawFlags {
    pub out: Option<String>,
    pub documents_only: bool,
    pub option: Option<String>,
    pub sheets: Option<String>,
    pub revision: Option<String>,
    pub date: Option<String>,
    pub style: Option<String>,
    pub section_spacing: Option<f64>,
    pub sheet_size: Option<String>,
    pub scale: Option<u32>,
    pub orthophoto: Option<String>,
    pub imagery: Option<String>,
    pub imagery_cache: Option<String>,
}

/// How long one imagery tile request may take before the fetch gives up (and the sheet falls back).
const IMAGERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The imagery tile cache: the flag, else `$XDG_CACHE_HOME/owd/imagery`, else `~/.cache/owd/imagery`.
fn imagery_cache(flag: Option<String>) -> std::path::PathBuf {
    flag.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache")))
            .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
            .join("owd")
            .join("imagery")
    })
}

fn from_draw(e: owd_format::drawing::DrawError) -> Failure {
    use owd_format::drawing::DrawError;
    let code = match e {
        DrawError::Input(_) | DrawError::Missing(_) => 2,
        DrawError::Checks(_) | DrawError::Format { .. } | DrawError::Layout { .. } => 1,
    };
    Failure {
        message: e.to_string(),
        code,
    }
}

pub fn draw(doc: &str, flags: DrawFlags, json_mode: bool) -> Result {
    use owd_format::drawing::{self, DrawOptions, SheetKind};
    let (session, bytes) = open_with_dem(doc)?;
    let mut options = DrawOptions {
        option: flags.option,
        date: flags.date,
        ..DrawOptions::default()
    };
    if let Some(list) = &flags.sheets {
        options.sheets = SheetKind::parse_list(list).map_err(from_draw)?;
    }
    if let Some(r) = flags.revision {
        options.revision = r;
    }
    let mut style = match &flags.style {
        Some(path) => drawing::Style::from_text(&read(path)?)
            .map_err(|e| input_error(format!("{path}: {e}")))?,
        None => drawing::Style::builtin(),
    };
    let mut overrides: Vec<(String, Value)> = Vec::new();
    if let Some(m) = flags.section_spacing {
        overrides.push(("sections.spacing_m".into(), json!(m)));
    }
    if let Some(z) = flags.sheet_size {
        overrides.push(("sheets.xs.size".into(), json!(z)));
    }
    if let Some(n) = flags.scale {
        overrides.push(("gallery.scales".into(), json!([n])));
    }
    style = style
        .with_overrides(&overrides)
        .map_err(|e| input_error(e.to_string()))?;
    options.style = style;
    let ortho = match &flags.orthophoto {
        Some(path) => {
            let data =
                std::fs::read(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))?;
            Some(
                owd_format::geotiff::read_rgb_bytes(&data)
                    .map_err(|e| input_error(format!("{path}: {e}")))?,
            )
        }
        None => None,
    };
    let source = flags
        .imagery
        .as_deref()
        .map(otd_imagery::Source::parse)
        .transpose()
        .map_err(|e| input_error(e.to_string()))?;
    let tiles = otd_imagery::Cached {
        dir: imagery_cache(flags.imagery_cache),
        inner: otd_imagery::Http::new(IMAGERY_TIMEOUT),
    };
    let background = match (&ortho, source) {
        (Some(rgb), _) => drawing::Background::Orthophoto(rgb),
        (None, Some(_)) => drawing::Background::Imagery(&tiles),
        (None, None) => drawing::Background::Relief,
    };
    let sheets =
        drawing::sheets_from_text_with(&session.to_text(), bytes.as_deref(), &options, &background)
            .map_err(from_draw)?;
    if flags.documents_only {
        let text = serde_json::to_string_pretty(&drawing::documents_json(&sheets))
            .map_err(|e| input_error(e.to_string()))?;
        println!("{text}");
        return Ok(());
    }
    let out = flags.out.unwrap_or_default();
    std::fs::create_dir_all(&out).map_err(|e| input_error(format!("cannot create {out}: {e}")))?;
    let mut written = Vec::new();
    let mut listed = Vec::new();
    for sheet in &sheets {
        let output = drawing::render(sheet).map_err(from_draw)?;
        for w in &output.warnings {
            eprintln!("owd: warning: sheet {}: {w}", output.number);
        }
        let mut names = Vec::new();
        for (name, data) in output.files() {
            let path = Path::new(&out).join(&name);
            std::fs::write(&path, data)
                .map_err(|e| input_error(format!("cannot write {}: {e}", path.display())))?;
            written.push(path.display().to_string());
            names.push(name);
        }
        listed.push(json!({"number": output.number, "files": names}));
    }
    say(
        false,
        json_mode,
        &written
            .iter()
            .map(|w| format!("wrote {w}"))
            .collect::<Vec<_>>()
            .join("\n"),
        json!({"ok": true, "sheets": listed}),
    );
    Ok(())
}

/// `owd draw run FILE`: one drawing document through the drawing crate; prints the result of clause
/// 17.3 as JSON. Used to compare the crate with the standard's Python reader.
pub fn draw_run(path: &str) -> Result {
    let text = read(path)?;
    // the files a document names (a plan's relief image) are next to it
    let folder = Path::new(path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let result = owd_format::drawing::run_document_with(&text, &FolderFiles(folder));
    println!("{}", result);
    Ok(())
}

/// The files of a drawing document, read from its folder.
struct FolderFiles(std::path::PathBuf);

impl otd::Files for FolderFiles {
    fn read(&self, relative_path: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(relative_path)).ok()
    }

    fn read_bytes(&self, relative_path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(relative_path)).ok()
    }
}

pub fn run(doc: &str, files: &[String], output: &Option<String>, json_mode: bool) -> Result {
    let (mut session, bytes) = open_with_dem(doc)?;
    load_rule_set_files(&mut session, files)?;
    // The DEM is used only when it is the file whose hash was recorded.
    let dem = match (session.dem_check(bytes.as_deref()), &bytes) {
        (DemCheck::Matches, Some(b)) => Some(read_dem(b, session.dem_path().unwrap_or("DEM"))?),
        _ => None,
    };
    let sampler = match &dem {
        Some(d) => Some(
            owd_format::design::terrain::dem_sampler(d, project_epsg(&session))
                .map_err(from_design)?,
        ),
        None => None,
    };
    let ran = session
        .run_stale_with(sampler.as_ref().map(|s| s as Terrain))
        .map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let names: Vec<&str> = ran.iter().map(|s| s.name()).collect();
    let human = if names.is_empty() {
        "nothing to run: every stage is fresh or not set up".to_string()
    } else {
        format!("ran: {}", names.join(", "))
    };
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "ran": names}),
    );
    Ok(())
}

pub fn validate(doc: &str, json_mode: bool) -> Result {
    owd_format::design::check_file_name(doc).map_err(from_design)?;
    let text = read(doc)?;
    let errors = match Session::open(&text) {
        Ok(_) => Vec::new(),
        Err(DesignError::Document(e)) => e,
        Err(other) => return Err(from_design(other)),
    };
    if errors.is_empty() {
        say(
            false,
            json_mode,
            "valid",
            json!({"valid": true, "errors": []}),
        );
        Ok(())
    } else {
        if json_mode {
            println!("{}", json!({"valid": false, "errors": errors}));
        } else {
            for e in &errors {
                println!("{e}");
            }
        }
        Err(Failure {
            message: format!("{} problem(s) found", errors.len()),
            code: 1,
        })
    }
}

pub fn status(doc: &str, json_mode: bool) -> Result {
    let (session, bytes) = open_with_dem(doc)?;
    let st = session.status_with_dem(bytes.as_deref());
    let mut human: Vec<String> = st
        .stages
        .iter()
        .map(|(s, status)| format!("{}: {}", s.name(), status.name()))
        .collect();
    let stale: Vec<&str> = st
        .stages
        .iter()
        .filter(|(_, s)| *s == StageStatus::Stale)
        .map(|(s, _)| s.name())
        .collect();
    if !stale.is_empty() {
        human.push(format!("stale: {}", stale.join(", ")));
    }
    // Each option's failed checks (spec 0002, spec 0005), and the water-aware note.
    let options: Vec<Value> = session.document()["options"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|o| {
            let failed = failed_checks(o);
            let note = water_note(o);
            if !failed.is_empty() {
                human.push(format!(
                    "option {}: FAILS {}",
                    o["id"].as_str().unwrap_or("?"),
                    failed.join(", ")
                ));
            }
            json!({"id": o["id"], "failed_checks": failed, "notes": note})
        })
        .collect();
    let mut stages = serde_json::Map::new();
    for (s, status) in &st.stages {
        stages.insert(
            s.name().into(),
            json!(match status {
                StageStatus::NotRun => "not_run",
                StageStatus::Fresh => "fresh",
                StageStatus::Stale => "stale",
            }),
        );
    }
    // The answer to "what fits" is not a stage: it shows only when the document holds one.
    let fits = session.fits_status();
    if fits != StageStatus::NotRun {
        human.push(format!("fits: {}", fits.name()));
    }
    let mut result = json!({
        "stages": Value::Object(stages),
        "stale": stale,
        "options": options,
        "warnings": session.dem_warning(bytes.as_deref()).into_iter().collect::<Vec<_>>(),
    });
    if fits != StageStatus::NotRun {
        result["fits"] = json!(if fits == StageStatus::Stale {
            "stale"
        } else {
            "fresh"
        });
    }
    say(false, json_mode, &human.join("\n"), result);
    Ok(())
}

pub fn schema() -> Result {
    write("-", Session::schema_text())
}

// --- reach (spec 0012) -------------------------------------------------------------------

/// The arguments of `owd reach`.
pub struct ReachFlags {
    pub banks: bool,
    pub jrc_tile: Vec<String>,
    pub mask_threshold: Option<f64>,
    pub edge_fit: bool,
    pub max_cloud: Option<f64>,
    pub stac: Option<String>,
    pub file: String,
    pub river: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub via: Vec<String>,
    pub trace: Option<String>,
    pub title: Option<String>,
    pub overpass: Option<String>,
    pub date: Option<String>,
    pub json: bool,
}

fn lon_lat(s: &str) -> core::result::Result<[f64; 2], Failure> {
    let bad = || input_error(format!("`{s}` is not `lon,lat` (for example -6.08,4.95)"));
    let (a, b) = s.split_once(',').ok_or_else(bad)?;
    let lon: f64 = a.trim().parse().map_err(|_| bad())?;
    let lat: f64 = b.trim().parse().map_err(|_| bad())?;
    if lon.abs() <= 180.0 && lat.abs() <= 90.0 {
        Ok([lon, lat])
    } else {
        Err(bad())
    }
}

/// The first line of a GeoJSON file (a LineString, or a Feature or collection holding one), as
/// longitude and latitude.
fn trace_line(text: &str, path: &str) -> core::result::Result<Vec<[f64; 2]>, Failure> {
    fn find(v: &Value) -> Option<&Value> {
        match v["type"].as_str()? {
            "LineString" => Some(&v["coordinates"]),
            "MultiLineString" => v["coordinates"].get(0),
            "Feature" => find(&v["geometry"]),
            "FeatureCollection" => v["features"].as_array()?.iter().find_map(find),
            _ => None,
        }
    }
    let v: Value = serde_json::from_str(text)
        .map_err(|e| input_error(format!("{path}: not valid JSON: {e}")))?;
    let coords = find(&v)
        .and_then(Value::as_array)
        .ok_or_else(|| input_error(format!("{path}: no LineString found")))?;
    coords
        .iter()
        .map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            input_error(format!(
                "{path}: the line has a point that is not [lon, lat]"
            ))
        })
}

pub fn reach(flags: ReachFlags) -> Result {
    let mut dropped = owd_reach::DroppedFiles::new();
    for path in &flags.jrc_tile {
        let bytes =
            std::fs::read(path).map_err(|e| input_error(format!("cannot read {path}: {e}")))?;
        let name = Path::new(path)
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        dropped = dropped.with_file(&name, bytes);
    }
    let http = crate::reach_fetch::HttpFetch;
    let jrc: &dyn owd_reach::Fetch = if flags.jrc_tile.is_empty() {
        &http
    } else {
        &dropped
    };
    reach_with(flags, &http, jrc)
}

/// `reach` with the fetchers given (Overpass, and the JRC tiles), so a test can serve recorded answers.
pub fn reach_with(
    flags: ReachFlags,
    fetch: &dyn owd_reach::Fetch,
    jrc: &dyn owd_reach::Fetch,
) -> Result {
    use owd_reach::overpass::{DEFAULT_ENDPOINT, Overpass};
    use owd_reach::reach::{plan_points, plan_trace, record_reach};
    let date = flags.date.clone().unwrap_or_else(today);
    if flags.start.is_none() && flags.trace.is_none() {
        return Err(input_error(
            "give --start and --end (and --river), or --trace FILE",
        ));
    }
    let ov = Overpass::with_endpoint(fetch, flags.overpass.as_deref().unwrap_or(DEFAULT_ENDPOINT));
    let river = flags.river.as_deref();
    let planned = if let Some(path) = &flags.trace {
        let line = trace_line(&read(path)?, path)?;
        plan_trace(&ov, river, &line)
    } else {
        let start = lon_lat(flags.start.as_deref().unwrap_or_default())?;
        let end = lon_lat(flags.end.as_deref().unwrap_or_default())?;
        let via = flags
            .via
            .iter()
            .map(|v| lon_lat(v))
            .collect::<core::result::Result<Vec<_>, _>>()?;
        plan_points(&ov, river, start, end, &via)
    }
    .map_err(|e| input_error(e.to_string()))?;
    let title = flags
        .title
        .clone()
        .or_else(|| flags.river.clone())
        .unwrap_or_else(|| "Reach study".to_string());
    if title.trim().is_empty() {
        return Err(input_error("the title must not be empty"));
    }
    let mut session = Session::new(&title, &date);
    record_reach(&mut session, &planned, &date).map_err(|e| input_error(e.to_string()))?;
    let banks = if flags.banks {
        use owd_reach::banks::{BankParams, banks_from_jrc, record_banks};
        let params = BankParams {
            threshold_pct: flags
                .mask_threshold
                .unwrap_or(BankParams::default().threshold_pct),
            ..BankParams::default()
        };
        if !(params.threshold_pct > 0.0 && params.threshold_pct <= 100.0) {
            return Err(input_error(
                "--mask-threshold must be a percentage above 0 and up to 100",
            ));
        }
        let b = banks_from_jrc(jrc, &planned.route.line(), &params)
            .map_err(|e| input_error(e.to_string()))?;
        record_banks(&mut session, &b).map_err(|e| input_error(e.to_string()))?;
        Some(b)
    } else {
        None
    };
    let fitted = match &banks {
        Some(b) if flags.edge_fit => {
            use owd_reach::edgefit::{
                FitParams, SceneQuery, default_range, fit_to_scene, record_edge_fit,
            };
            use owd_reach::sentinel2::{
                DEFAULT_MAX_CLOUD_PCT, SEARCH_ENDPOINT, default_dry_months,
            };
            let max_cloud = flags.max_cloud.unwrap_or(DEFAULT_MAX_CLOUD_PCT);
            if !(0.0..=100.0).contains(&max_cloud) {
                return Err(input_error(
                    "--max-cloud must be a percentage from 0 to 100",
                ));
            }
            let (from, to) = default_range(&date).map_err(|e| input_error(e.to_string()))?;
            let line = planned.route.line();
            let q = SceneQuery {
                endpoint: flags
                    .stac
                    .clone()
                    .unwrap_or_else(|| SEARCH_ENDPOINT.to_string()),
                from,
                to,
                months: default_dry_months(line[line.len() / 2][1]),
                max_cloud_pct: max_cloud,
            };
            let f = fit_to_scene(fetch, b, &FitParams::default(), &q)
                .map_err(|e| input_error(e.to_string()))?;
            record_edge_fit(&mut session, &f).map_err(|e| input_error(e.to_string()))?;
            Some(f)
        }
        _ => None,
    };
    write(&flags.file, &session.to_text())?;
    let route = &planned.route;
    let ways = route.way_ids().len();
    let drawn = route.as_drawn_count();
    let found = planned.candidates.first().map(|c| {
        format!(
            "{} {} way(s) named \"{}\"",
            planned
                .candidates
                .iter()
                .map(|c| c.way_ids.len())
                .sum::<usize>(),
            c.kind,
            c.name
        )
    });
    let mut human = format!(
        "wrote {}: route of {:.1} km over {ways} way(s)",
        flags.file,
        route.length_m / 1000.0
    );
    if drawn > 0 {
        human.push_str(&format!(
            "; {drawn} stretch(es) kept as drawn, with no waterway near (check them)"
        ));
    }
    if let Some(b) = &banks {
        human.push_str(&format!(
            "\nbanks, polygon and centreline from the JRC water mask ({}% threshold, a provisional default): {} station(s), {} low-confidence stretch(es) to check",
            b.params.threshold_pct,
            b.widths.len(),
            b.stretches.len()
        ));
        human.push_str("\nSource: EC JRC/Google; Pekel et al. 2016, Nature 540, 418-422");
    }
    if let Some(f) = &fitted {
        let moved = f
            .fit
            .left_shift
            .iter()
            .chain(&f.fit.right_shift)
            .filter(|d| d.abs() > 0.5)
            .count();
        let all = f.fit.left_shift.len() + f.fit.right_shift.len();
        human.push_str(&format!(
            "\nbanks moved onto the water edge in Sentinel-2 scene {} ({}, {:.1}% cloud): {moved} of {all} bank station(s) moved, at most {} m (a provisional default); {} stretch(es) to check",
            f.scene.id, f.scene.date, f.scene.cloud_pct, f.fit.params.max_shift_m, f.fit.stretches.len()
        ));
        human.push_str(&format!(
            "\nContains modified Copernicus Sentinel data {}",
            &f.scene.date[..4]
        ));
    }
    human.push_str("\n© OpenStreetMap contributors (ODbL)");
    say(
        flags.file == "-",
        flags.json,
        &human,
        json!({
            "ok": true, "file": flags.file, "length_m": route.length_m,
            "ways": route.way_ids(), "as_drawn_stretches": drawn,
            "river_search": found,
            "banks": banks.as_ref().map(|b| json!({
                "mask_threshold_pct": b.params.threshold_pct,
                "stations": b.widths.len(),
                "low_confidence_stretches": b.stretches.len(),
            })),
            "edge_fit": fitted.as_ref().map(|f| json!({
                "scene_id": f.scene.id,
                "scene_date": f.scene.date,
                "scene_cloud_pct": f.scene.cloud_pct,
                "max_shift_m": f.fit.params.max_shift_m,
                "largest_shift_m": f.fit.left_shift.iter().chain(&f.fit.right_shift).fold(0.0f64, |a, d| a.max(d.abs())),
                "low_confidence_stretches": f.fit.stretches.len(),
            })),
        }),
    );
    Ok(())
}

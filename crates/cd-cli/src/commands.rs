//! The `cdesign` subcommands. File, stdin, stdout and clock access live here.

use cd_format::design::crs::{Crs, Utm};
use cd_format::design::geopackage;
use cd_format::design::{AlignMethod, DesignError, Role, Session, Stage, StageStatus};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::Path;

/// A failed command: its message and exit code (1 a failed check, 2 a usage or input error).
pub struct Failure {
    pub message: String,
    pub code: u8,
}

pub type Result = core::result::Result<(), Failure>;

fn input_error(message: impl Into<String>) -> Failure {
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

fn from_design(e: DesignError) -> Failure {
    match e {
        DesignError::StaleUpstream { stage, run_first } => Failure {
            message: format!(
                "the {} stage cannot run: {} is stale. Run `cdesign {}` first (or `cdesign run`).",
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

fn write(path: &str, text: &str) -> Result {
    if path == "-" {
        std::io::stdout()
            .write_all(text.as_bytes())
            .map_err(|e| input_error(format!("cannot write stdout: {e}")))
    } else {
        std::fs::write(path, text).map_err(|e| input_error(format!("cannot write {path}: {e}")))
    }
}

fn open(path: &str) -> core::result::Result<Session, Failure> {
    Session::open(&read(path)?).map_err(from_design)
}

/// Print a result. When the document itself goes to stdout the result goes to stderr, so the two
/// never mix.
fn say(doc_on_stdout: bool, json_mode: bool, human: &str, value: Value) {
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
fn destination(doc: &str, output: &Option<String>) -> String {
    output.clone().unwrap_or_else(|| doc.to_string())
}

/// Today's date in UTC, `YYYY-MM-DD`, from the system clock.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    cd_format::design::date_from_unix(secs)
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
    if texts.is_empty() {
        return Err(input_error(
            "nothing to import: give --left-bank, --right-bank, --centreline or --polygon",
        ));
    }
    // The document is read first when it is on stdin: stdin can be read once.
    let mut session = open(doc)?;
    let items: Vec<(Role, &str)> = texts.iter().map(|(r, t)| (*r, t.as_str())).collect();
    let report = session.import(&items).map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let human = {
        let mut lines = vec![format!("project CRS: EPSG:{}", report.project_epsg)];
        for (name, src, n) in &report.inputs {
            lines.push(format!("{name}: {n} positions, from {src}"));
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

pub fn size(
    doc: &str,
    ruleset: &str,
    vessel: &str,
    draught: Option<f64>,
    output: &Option<String>,
    json_mode: bool,
) -> Result {
    // Opened first: when the document comes from stdin it must be read before anything else.
    let mut session = open(doc)?;
    session
        .set_sizing(ruleset, vessel, draught)
        .map_err(from_design)?;
    session.run_stage(Stage::Sizing).map_err(from_design)?;
    let dest = destination(doc, output);
    write(&dest, &session.to_text())?;
    let shape = session.document()["stages"]["sizing"]["results"].clone();
    let f = |k: &str| shape[k].as_f64().unwrap_or(f64::NAN);
    let mut human = format!(
        "{} x {}: bottom width {:.1} m, R_min {:.0} m, design depth {:.2} m",
        ruleset,
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
    say(
        dest == "-",
        json_mode,
        &human,
        json!({"ok": true, "sizing": shape}),
    );
    Ok(())
}

/// The flags of `cdesign align`.
pub struct AlignFlags {
    pub step: Option<f64>,
    pub passes: Option<u32>,
    pub chord: Option<f64>,
    pub dem: Option<String>,
    pub water_level: Option<f64>,
    pub water_slope: Option<f64>,
    pub corridors: Option<String>,
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
            if let Some(v) = flags.water_level {
                put("water_level_start_m", json!(v));
            }
            if let Some(v) = flags.water_slope {
                put("water_surface_slope", json!(v));
            }
            if let Some(list) = &flags.corridors {
                let parsed: core::result::Result<Vec<f64>, _> =
                    list.split(',').map(|x| x.trim().parse::<f64>()).collect();
                let parsed = parsed
                    .map_err(|_| input_error(format!("--corridors must be numbers: `{list}`")))?;
                put("corridor_sweep_m", json!(parsed));
            }
        }
    }
    if let Some(v) = flags.chord {
        put("check_chord_m", json!(v));
    }
    let overrides = Value::Object(overrides);
    // The re-route samples a DEM; read it here, since the session has no file access.
    let dem = match (&flags.dem, method) {
        (Some(path), AlignMethod::Reroute) => Some(
            cd_format::geotiff::read_file(Path::new(path))
                .map_err(|e| input_error(format!("{path}: {e}")))?,
        ),
        (Some(_), _) => return Err(input_error("--dem is for the re-route only")),
        _ => None,
    };
    let project = session.document()["project"]["crs"]["epsg"]
        .as_u64()
        .and_then(|c| Crs::from_epsg(c as u32).ok());
    let sampler = dem.as_ref().map(|d| {
        let to_dem: Box<dyn Fn(f64, f64) -> [f64; 2]> = match (d.epsg, project) {
            (Some(4326), Some(Crs::Utm(u))) => {
                Box::new(move |x, y| u.inverse(x, y).unwrap_or([f64::NAN; 2]))
            }
            _ => Box::new(|x, y| [x, y]),
        };
        move |xs: &[f64], ys: &[f64]| -> Vec<f64> {
            xs.iter()
                .zip(ys)
                .map(|(&x, &y)| {
                    let p = to_dem(x, y);
                    d.sample_bilinear(p[0], p[1])
                })
                .collect()
        }
    });
    if let (Some(d), Some(Crs::Utm(Utm { zone, .. }))) = (&dem, project)
        && !matches!(d.epsg, Some(4326))
        && d.epsg != project.map(Crs::epsg)
    {
        return Err(input_error(format!(
            "the DEM's CRS ({}) is not WGS84 and not the channel design's UTM zone {zone}. Convert the DEM.",
            d.epsg
                .map_or("unknown".to_string(), |e| format!("EPSG:{e}"))
        )));
    }
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
    let ok = c["radius_ok"].as_bool().unwrap_or(false);
    let human = format!(
        "{}: minimum radius {:.0} m on a {:.0} m chord against R_min {:.0} m: {}",
        option["name"].as_str().unwrap_or(method.name()),
        c["measured_min_radius_m"].as_f64().unwrap_or(f64::NAN),
        c["radius_check_chord_m"].as_f64().unwrap_or(f64::NAN),
        c["r_min_m"].as_f64().unwrap_or(f64::NAN),
        if ok { "ok" } else { "FAILS R_min" }
    );
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
            message: "the option fails R_min".into(),
            code: 1,
        })
    }
}

pub fn run(doc: &str, output: &Option<String>, json_mode: bool) -> Result {
    let mut session = open(doc)?;
    let ran = session.run_stale().map_err(from_design)?;
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
    let session = open(doc)?;
    let st = session.status();
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
    say(
        false,
        json_mode,
        &human.join("\n"),
        json!({"stages": Value::Object(stages), "stale": stale}),
    );
    Ok(())
}

pub fn schema() -> Result {
    write("-", Session::schema_text())
}

pub fn eval(entry_point: &str) -> Result {
    let text = read("-")?;
    let input: Value =
        serde_json::from_str(&text).map_err(|e| input_error(format!("stdin is not JSON: {e}")))?;
    let output = cd_format::eval(entry_point, &input).map_err(|e| input_error(e.to_string()))?;
    println!("{output}");
    Ok(())
}
